fn main() {
    // Windows 上，测试二进制缺少 comctl32 v6 的 side-by-side manifest。
    //
    // 现象：集成测试（含 `tauri::test` 的 IPC 测试）链接了
    // `comctl32.dll!TaskDialogIndirect`——该导出只有 comctl32 v6（SxS）才有。
    // `tauri-build` 只为**主程序**嵌入 v6 manifest，测试二进制没有，
    // 于是加载器把 `comctl32.dll` 解析到 System32 的 v5.82，
    // 找不到该导出 → 进程以 STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139) 退出，
    // 测试根本没跑起来就崩了。
    //
    // 办法：把 comctl32 改成**延迟加载**。这样加载期不再要求该导出；
    // 真正调用时才会解析。测试里的 mock 运行时不会弹 TaskDialog，
    // 因此永远走不到那个调用点。
    //
    // 只对 Windows 生效；其它平台没有这个问题。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-tests=/DELAYLOAD:comctl32.dll");
        // /DELAYLOAD 需要 delayimp.lib 里的 __delayLoadHelper2。
        println!("cargo:rustc-link-lib=delayimp");
    }

    tauri_build::build()
}
