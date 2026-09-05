fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", "BUTION Distributed Local AI Cluster");
        res.set("ProductName", "BUTION");
        res.set("OriginalFilename", "bution.exe");
        res.set("LegalCopyright", "Copyright (c) 2025 BUTION Contributors");
        let _ = res.compile();
    }
}
