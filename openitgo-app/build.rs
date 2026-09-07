fn main() {
    // Windows：把 exe 图标资源嵌进二进制（资源脚本见 assets/icon/openitgo.rc）；
    // manifest_required 在资源未嵌入（NotAttempted/Failed）时直接报错，防止静默出无图标的包
    #[cfg(target_os = "windows")]
    embed_resource::compile("../assets/icon/openitgo.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
