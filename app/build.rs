// Gives the Windows exe its icon. Linux has no equivalent, so there's nothing to do there.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/StrataMon.rc");
        println!("cargo:rerun-if-changed=assets/StrataMon.ico");
        embed_resource::compile("assets/StrataMon.rc", embed_resource::NONE).manifest_required().expect("the app icon should compile");
    }
}
