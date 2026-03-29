use std::borrow::Cow;

use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph, Span};
use iced::advanced::widget::text::Catalog;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::mouse;
use iced::widget::text::Wrapping;
use iced::{Color, Element, Event, Length, Pixels, Point, Rectangle, Size};

use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq)]
struct RawSelection {
    start: Point,
    end: Point,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedSelection {
    start: Point,
    end: Point,
}

#[derive(Debug, Clone, Copy)]
struct CharSelection {
    start: usize,
    end: usize,
}

impl RawSelection {
    fn resolve(self, bounds: Rectangle) -> Option<ResolvedSelection> {
        let (mut start, mut end) = (self.start, self.end);

        // Order top-to-bottom, then left-to-right on same line.
        if start.y > end.y || (start.y == end.y && start.x > end.x) {
            std::mem::swap(&mut start, &mut end);
        }

        // Check Y range intersection with widget bounds.
        if end.y < bounds.y || start.y > bounds.y + bounds.height {
            return None;
        }

        // Clip to bounds.
        start.x = start.x.clamp(bounds.x, bounds.x + bounds.width);
        start.y = start.y.clamp(bounds.y, bounds.y + bounds.height);
        end.x = end.x.clamp(bounds.x, bounds.x + bounds.width);
        end.y = end.y.clamp(bounds.y, bounds.y + bounds.height);

        // Require at least 1px horizontal difference for single-line selections.
        if (start.y - end.y).abs() < 1.0 && (end.x - start.x).abs() < 1.0 {
            return None;
        }

        Some(ResolvedSelection { start, end })
    }
}

fn find_cursor_position<P: Paragraph>(
    paragraph: &P,
    total_text: &str,
    point: Point,
) -> Option<usize> {
    let hit = paragraph.hit_test(point)?;
    let byte_offset = hit.cursor();
    let clamped = byte_offset.min(total_text.len());
    Some(UnicodeSegmentation::graphemes(&total_text[..clamped], true).count())
}

fn resolve_char_selection<P: Paragraph>(
    raw: RawSelection,
    bounds: Rectangle,
    paragraph: &P,
    total_text: &str,
) -> Option<CharSelection> {
    let resolved = raw.resolve(bounds)?;

    let start_local = Point::new(resolved.start.x - bounds.x, resolved.start.y - bounds.y);
    let end_local = Point::new(resolved.end.x - bounds.x, resolved.end.y - bounds.y);

    let start = find_cursor_position(paragraph, total_text, start_local)?;
    let end = find_cursor_position(paragraph, total_text, end_local)?;

    if start == end {
        return None;
    }

    Some(CharSelection {
        start: start.min(end),
        end: start.max(end),
    })
}

fn select_graphemes(text: &str, start: usize, end: usize) -> &str {
    let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(text, true).collect();
    let start_idx = start.min(graphemes.len());
    let end_idx = end.min(graphemes.len());
    if start_idx >= end_idx || graphemes.is_empty() {
        return "";
    }

    let byte_start: usize = graphemes[..start_idx].iter().map(|g| g.len()).sum();
    let byte_end: usize = graphemes[..end_idx].iter().map(|g| g.len()).sum();
    &text[byte_start..byte_end]
}

/// Maximum distance (in pixels) between press and release to count as a click
/// rather than a drag selection.
const CLICK_THRESHOLD: f32 = 3.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Interaction {
    #[default]
    Idle,
    Selecting(RawSelection),
    Selected(RawSelection),
}

impl Interaction {
    fn raw(self) -> Option<RawSelection> {
        match self {
            Interaction::Idle => None,
            Interaction::Selecting(raw) | Interaction::Selected(raw) => Some(raw),
        }
    }
}

struct State<P: Paragraph> {
    spans: Vec<Span<'static, String, P::Font>>,
    paragraph: P,
    interaction: Interaction,
}

pub struct SelectableRichText<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    spans: Cow<'a, [Span<'a, String, Renderer::Font>]>,
    size: Option<Pixels>,
    width: Length,
    font: Option<Renderer::Font>,
    wrapping: Wrapping,
    class: Theme::Class<'a>,
    selection_color: Color,
    on_link_click: Option<Box<dyn Fn(String) -> Message + 'a>>,
    on_right_click: Option<Box<dyn Fn(Point, Option<String>) -> Message + 'a>>,
}

impl<'a, Message, Theme, Renderer> SelectableRichText<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
    Renderer::Font: 'a,
{
    pub fn new(spans: Vec<Span<'a, String, Renderer::Font>>) -> Self {
        Self {
            spans: Cow::Owned(spans),
            size: None,
            width: Length::Shrink,
            font: None,
            wrapping: Wrapping::default(),
            class: Theme::default(),
            selection_color: Color::from_rgba8(0x3f, 0x3f, 0x3f, 1.0),
            on_link_click: None,
            on_right_click: None,
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }

    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = color;
        self
    }

    pub fn on_link_click(mut self, handler: impl Fn(String) -> Message + 'a) -> Self {
        self.on_link_click = Some(Box::new(handler));
        self
    }

    pub fn on_right_click(
        mut self,
        handler: impl Fn(Point, Option<String>) -> Message + 'a,
    ) -> Self {
        self.on_right_click = Some(Box::new(handler));
        self
    }

    fn total_text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_ref()).collect()
    }

    fn span_has_link(&self, span_index: usize) -> Option<&str> {
        self.spans.get(span_index).and_then(|s| s.link.as_deref())
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for SelectableRichText<'_, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Renderer::Paragraph> {
            spans: Vec::new(),
            paragraph: Renderer::Paragraph::default(),
            interaction: Interaction::Idle,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Shrink,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();

        layout::sized(limits, self.width, Length::Shrink, |limits| {
            let bounds = limits.max();
            let size = self.size.unwrap_or_else(|| renderer.default_size());
            let font = self.font.unwrap_or_else(|| renderer.default_font());

            let text_with_spans = || iced::advanced::text::Text {
                content: self.spans.as_ref(),
                bounds,
                size,
                line_height: text::LineHeight::default(),
                font,
                align_x: text::Alignment::Default,
                align_y: iced::alignment::Vertical::Top,
                shaping: text::Shaping::Advanced,
                wrapping: self.wrapping,
            };

            if state.spans != *self.spans.as_ref() {
                state.paragraph = Renderer::Paragraph::with_spans(text_with_spans());
                state.spans = self.spans.iter().cloned().map(Span::to_static).collect();
            } else {
                match state.paragraph.compare(iced::advanced::text::Text {
                    content: (),
                    bounds,
                    size,
                    line_height: text::LineHeight::default(),
                    font,
                    align_x: text::Alignment::Default,
                    align_y: iced::alignment::Vertical::Top,
                    shaping: text::Shaping::Advanced,
                    wrapping: self.wrapping,
                }) {
                    text::Difference::None => {}
                    text::Difference::Bounds => {
                        state.paragraph.resize(bounds);
                    }
                    text::Difference::Shape => {
                        state.paragraph = Renderer::Paragraph::with_spans(text_with_spans());
                    }
                }
            }

            state.paragraph.min_bounds()
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        if !bounds.intersects(viewport) {
            return;
        }

        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();

        // Draw selection highlight behind the text.
        if let Some(raw) = state.interaction.raw()
            && let Some(resolved) = raw.resolve(bounds)
        {
            let size = self.size.unwrap_or_else(|| renderer.default_size());
            let line_height = text::LineHeight::default().to_absolute(size);

            let baseline_y =
                bounds.y + ((resolved.start.y - bounds.y) / line_height.0).floor() * line_height.0;
            let rows = ((resolved.end.y - baseline_y + 0.5) / line_height.0)
                .ceil()
                .max(1.0) as usize;

            for row in 0..rows {
                let row_y = baseline_y + row as f32 * line_height.0;

                let (x_start, x_end) = if rows == 1 {
                    (resolved.start.x, resolved.end.x)
                } else if row == 0 {
                    (resolved.start.x, bounds.x + bounds.width)
                } else if row == rows - 1 {
                    (bounds.x, resolved.end.x)
                } else {
                    (bounds.x, bounds.x + bounds.width)
                };

                let highlight_bounds = Rectangle {
                    x: x_start,
                    y: row_y,
                    width: (x_end - x_start).max(0.0),
                    height: line_height.0,
                };

                renderer.fill_quad(
                    renderer::Quad {
                        bounds: highlight_bounds,
                        ..Default::default()
                    },
                    self.selection_color,
                );
            }
        }

        // Draw the text on top.
        let style = theme.style(&self.class);
        iced::advanced::widget::text::draw(
            renderer,
            defaults,
            bounds,
            &state.paragraph,
            style,
            viewport,
        );

        // Draw underlines for spans that request them (e.g., URL links).
        // iced's text::draw helper renders glyphs but not decorations,
        // so we draw them manually like iced's built-in Rich widget does.
        let translation = bounds.position() - Point::ORIGIN;
        for (index, span) in self.spans.iter().enumerate() {
            if !span.underline {
                continue;
            }
            let regions = state.paragraph.span_bounds(index);
            let span_size = span.size.unwrap_or_else(|| renderer.default_size());
            let span_line_height = text::LineHeight::default().to_absolute(span_size);
            let color = span.color.unwrap_or(defaults.text_color);
            let baseline_offset = iced::Vector::new(
                0.0,
                span_size.0 + (span_line_height.0 - span_size.0) / 2.0 - span_size.0 * 0.08,
            );
            for region in &regions {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle::new(
                            region.position() + translation + baseline_offset,
                            Size::new(region.width, 1.0),
                        ),
                        ..Default::default()
                    },
                    color,
                );
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let bounds = layout.bounds();

        let old_interaction = state.interaction;

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
                if let Some(position) = cursor.position() {
                    if bounds.contains(position) {
                        state.interaction = Interaction::Selecting(RawSelection {
                            start: position,
                            end: position,
                        });
                        shell.capture_event();
                    } else {
                        state.interaction = Interaction::Idle;
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(iced::touch::Event::FingerMoved { .. }) => {
                if let Interaction::Selecting(ref mut raw) = state.interaction
                    && let Some(position) = cursor.position()
                {
                    raw.end = position;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(iced::touch::Event::FingerLifted { .. }) => {
                if let Interaction::Selecting(raw) = state.interaction {
                    let distance = ((raw.end.x - raw.start.x).powi(2)
                        + (raw.end.y - raw.start.y).powi(2))
                    .sqrt();

                    if distance < CLICK_THRESHOLD {
                        // This was a click, not a drag. Check for link.
                        if let Some(on_link_click) = &self.on_link_click {
                            let local = Point::new(raw.start.x - bounds.x, raw.start.y - bounds.y);
                            if let Some(span_index) = state.paragraph.hit_span(local) {
                                if let Some(url) = self.span_has_link(span_index) {
                                    shell.publish((on_link_click)(url.to_string()));
                                }
                            }
                        }
                        state.interaction = Interaction::Idle;
                    } else {
                        state.interaction = Interaction::Selected(raw);
                    }
                } else if matches!(state.interaction, Interaction::Selected(_))
                    && cursor.position().is_none_or(|p| !bounds.contains(p))
                {
                    state.interaction = Interaction::Idle;
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(on_right_click) = &self.on_right_click
                    && let Some(position) = cursor.position()
                    && bounds.contains(position)
                {
                    let local = Point::new(position.x - bounds.x, position.y - bounds.y);
                    let link_url = state
                        .paragraph
                        .hit_span(local)
                        .and_then(|idx| self.span_has_link(idx))
                        .map(String::from);
                    shell.publish((on_right_click)(position, link_url));
                    shell.capture_event();
                }
            }
            _ => {}
        }

        if state.interaction != old_interaction {
            shell.request_redraw();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let bounds = layout.bounds();
        if let Some(position) = cursor.position() {
            if bounds.contains(position) {
                if self.on_link_click.is_some() {
                    let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
                    let local = Point::new(position.x - bounds.x, position.y - bounds.y);
                    if let Some(span_index) = state.paragraph.hit_span(local) {
                        if self.span_has_link(span_index).is_some() {
                            return mouse::Interaction::Pointer;
                        }
                    }
                }
                return mouse::Interaction::Text;
            }
        }
        mouse::Interaction::None
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let bounds = layout.bounds();

        let total_text = self.total_text();

        if let Some(raw) = state.interaction.raw()
            && let Some(sel) = resolve_char_selection(raw, bounds, &state.paragraph, &total_text)
        {
            let selected = select_graphemes(&total_text, sel.start, sel.end).to_string();
            operation.custom(None, bounds, &mut selected.into_boxed_str());
        }
    }
}

impl<'a, Message, Theme, Renderer> From<SelectableRichText<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(widget: SelectableRichText<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}

pub fn selected<Message: Send + 'static>(
    callback: fn(Vec<(f32, String)>) -> Message,
) -> iced::Task<Message> {
    struct SelectedOp<T> {
        contents: Vec<(f32, String)>,
        callback: fn(Vec<(f32, String)>) -> T,
    }

    impl<T> iced::advanced::widget::Operation<T> for SelectedOp<T> {
        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<T>),
        ) {
            operate(self);
        }

        fn container(&mut self, _id: Option<&iced::advanced::widget::Id>, _bounds: Rectangle) {}

        fn custom(
            &mut self,
            _id: Option<&iced::advanced::widget::Id>,
            bounds: Rectangle,
            state: &mut dyn std::any::Any,
        ) {
            if let Some(content) = state.downcast_ref::<Box<str>>()
                && !content.is_empty()
            {
                self.contents.push((bounds.y, content.to_string()));
            }
        }

        fn finish(&self) -> iced::advanced::widget::operation::Outcome<T> {
            iced::advanced::widget::operation::Outcome::Some((self.callback)(self.contents.clone()))
        }
    }

    iced::advanced::widget::operate(SelectedOp {
        contents: vec![],
        callback,
    })
}
