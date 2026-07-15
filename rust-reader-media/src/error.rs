use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("无法初始化播放器：{0}")]
    Init(String),
    #[error("播放器命令失败（{code}）：{what}")]
    Command { code: i32, what: String },
    #[error("无法播放该文件：{0}")]
    Load(String),
}
