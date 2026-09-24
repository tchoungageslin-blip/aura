//! Build script — embeds the Windows application icon (assets/icon/aura.ico)
//! into aura.exe via the SDK resource compiler. No-op on other OSes.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        match embed_resource::compile("icon.rc", embed_resource::NONE) {
            embed_resource::CompilationResult::Failed(e) => {
                panic!("failed to embed icon: {e} (need rc.exe / llvm-rc)")
            }
            _ => {}
        }
    }
}
