fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/scope2000.ico");
        resource.compile().expect("compile Windows resources");
    }
}
