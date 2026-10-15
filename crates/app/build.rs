// Embeds the Tessera icon and version info into the Windows executable.
fn main() {
    println!("cargo:rerun-if-changed=assets/tessera.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/tessera.ico");
        res.set("ProductName", "Tessera");
        res.set("FileDescription", "Tessera API client");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the app icon: {e}");
        }
    }
}
