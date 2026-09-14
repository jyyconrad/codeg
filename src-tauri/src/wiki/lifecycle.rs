//! 串行化 Wiki 目录切换、素材登记和任务认领的短临界区。
//! settings/source/engine/read_model 使用同一锁，防止一次操作混用两个目录身份。
//! 锁不跨模型执行或用户界面会话持有；文件内容写入另由 commit 的文件锁保护。

use tokio::sync::{Mutex, MutexGuard};
static TRANSITION: Mutex<()> = Mutex::const_new(());

pub async fn lock() -> MutexGuard<'static, ()> {
    TRANSITION.lock().await
}
