//! Windows metadata for the CLI binary.
//!
//! A release build without a version resource and an application manifest looks
//! like an anonymous executable to Windows: Explorer shows no product details,
//! and heuristic scanners score it higher. This embeds the same metadata the
//! Desktop bundle carries, plus an `asInvoker` manifest so the CLI never asks
//! for elevation.
//!
//! The build is a no-op on non-Windows hosts, so the Linux CI job is unaffected.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        // The assembly identity needs a four-part version, and it must follow
        // the crate version so a release bump cannot leave a stale manifest.
        let version = env!("CARGO_PKG_VERSION");
        let assembly_version = if version.matches('.').count() == 2 {
            format!("{version}.0")
        } else {
            version.to_string()
        };
        let manifest = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="dbpinball.cli" version="{assembly_version}" processorArchitecture="*"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>"#
        );

        let mut resource = winresource::WindowsResource::new();
        resource
            .set_manifest(&manifest)
            .set("ProductName", "DBPinball CLI")
            .set("FileDescription", "DBPinball command-line interface")
            .set("OriginalFilename", "dbpinball.exe")
            .set("LegalCopyright", "MIT licensed")
            .set("CompanyName", "DBPinball")
            .set("ProductVersion", env!("CARGO_PKG_VERSION"))
            .set("FileVersion", env!("CARGO_PKG_VERSION"));

        if let Err(error) = resource.compile() {
            // The version resource is what keeps the release binary from being
            // quarantined by heuristic scanners, so a Windows build must not
            // silently ship without it.
            panic!("Windows resource compilation failed: {error}");
        }
    }
}
