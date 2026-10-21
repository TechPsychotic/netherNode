//! Embeds the app icon and version info into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/nethernode.ico");
    let target_windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    // The resource compiler only exists on Windows hosts; cross builds just skip the icon.
    let host_windows = std::env::var("HOST").is_ok_and(|h| h.contains("windows"));
    if target_windows && host_windows {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/nethernode.ico")
            .set("ProductName", "netherNode")
            .set("FileDescription", "netherNode Minecraft launcher");
        res.compile().expect("failed to embed the Windows icon");
    }
}
