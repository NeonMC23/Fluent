fn main() {
    println!("cargo:rerun-if-changed=../../src/logo/generated/windows/fluent.ico");

    #[cfg(windows)]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("../../src/logo/generated/windows/fluent.ico");
        resource.set("ProductName", "Fluent");
        resource.set("FileDescription", "Fluent - Local Translation");
        resource.set("OriginalFilename", "fluent.exe");
        resource.set("CompanyName", "Fluent");

        let mut parts = env!("CARGO_PKG_VERSION")
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0));
        let version = (parts.next().unwrap_or(0) << 48)
            | (parts.next().unwrap_or(0) << 32)
            | (parts.next().unwrap_or(0) << 16);
        resource.set_version_info(winres::VersionInfo::FILEVERSION, version);
        resource.set_version_info(winres::VersionInfo::PRODUCTVERSION, version);

        resource
            .compile()
            .expect("failed to embed the Fluent Windows icon resource");
    }
}
