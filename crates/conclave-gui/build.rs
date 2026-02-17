fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/conclave.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
