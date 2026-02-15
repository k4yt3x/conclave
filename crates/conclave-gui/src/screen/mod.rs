pub mod dashboard;
pub mod login;

pub use dashboard::Dashboard;
pub use login::Login;

pub enum Screen {
    Login(Login),
    Dashboard(Dashboard),
}
