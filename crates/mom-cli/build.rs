// mom.exe's version information: the product it belongs to, its version and its icon. Windows
// shows them (in the prompt asking whether it may run as administrator, among other places), and
// code signing checks them.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        let icon = "../../app/src-tauri/icons/icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        let mut res = tauri_winres::WindowsResource::new();
        res.set("ProductName", "momputer")
            .set("FileDescription", "mom, the momputer command line")
            .set("CompanyName", "momputer contributors")
            .set("LegalCopyright", "Copyright (c) 2026 Ilya Arbabi")
            .set("InternalName", "mom")
            .set("OriginalFilename", "mom.exe")
            .set_icon(icon)
            .set_manifest(MANIFEST);
        res.compile().expect("couldn't add mom.exe's version information");
    }
}

/// Runs as whoever starts it (never asks for an administrator by itself), on Windows 10 and later.
#[cfg(windows)]
const MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" />
    </application>
  </compatibility>
</assembly>"#;
