fn main() {
    println!("cargo:rerun-if-changed=resources/app.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("resources/app.ico")
            .set("ProductName", "Agent Vocal Studio")
            .set("FileDescription", "Agent Vocal Studio 音乐编辑器")
            .compile()
            .expect("Windows application resources");
    }
}
