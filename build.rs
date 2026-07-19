fn main() {
    println!("cargo:rerun-if-changed=xdg/io.github.ergolyam.Drosophila.ico");

    #[cfg(windows)]
    {
        let manifest = r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>
"#;

        winresource::WindowsResource::new()
            .set_icon("xdg/io.github.ergolyam.Drosophila.ico")
            .set_manifest(manifest)
            .compile()
            .expect("failed to compile Windows resources");
    }
}
