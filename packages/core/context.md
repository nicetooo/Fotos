跨桌面 + 移动照片管理 Rust Core（强约束）

你正在编写一个 跨 macOS / Windows / Linux / iOS / Android 的照片管理软件的 Rust 核心引擎（library）。

一、角色与定位（不可违反）

这是 纯 Rust 引擎库，不是应用

必须 平台无关

必须可被桌面 / 移动端 / WebView 外壳复用

如果某个设计在 iOS 上不成立，它就不属于 Rust Core。

二、允许的职责（只做这些）

文件遍历（PathBuf）

图片解码 / 转码（JPEG / PNG / HEIC / AVIF）

EXIF / 元数据解析

缩略图生成（写入磁盘）

Hash / 感知 Hash / 去重

SQLite 索引与查询

缓存 / 批处理 / 算法策略

三、绝对禁止（出现即错误）

❌ 平台 API（AppKit / Win32 / Android SDK / Photos.framework 等）

❌ 平台判断（cfg!(target_os) 业务分支）

❌ UI / 交互逻辑

❌ 权限管理

❌ async/await

❌ 线程池 / 复杂并发

❌ 全局状态 / 单例

❌ 回调 / 闭包作为 API 输入

❌ 返回大图内存 / Base64

四、输入 / 输出边界（必须遵守）
输入只允许：

PathBuf

Vec<u8>

String

纯数据 Config / ID

输出只允许：

Result<T, CoreError>

PathBuf

纯数据结构（struct / enum）

五、错误与稳定性规则

不允许 unwrap / expect

不允许 panic（除非不可恢复）

所有失败必须显式返回 Result

六、模块设计规则

单一职责

模块之间仅通过函数参数通信

不共享状态

不暴露生命周期参数

不暴露 trait 对象或泛型到 API

七、FFI / IPC 预留（不实现）

API 必须 FFI-friendly

只用基础类型

不做序列化或 IPC 逻辑

八、输出风格约束

优先输出 Rust 代码

设计若违反以上约束，指出问题而不是实现

不引入额外抽象或“架构升级”

请严格遵守以上边界生成代码。

九、路径与数据库契约（Path & Database Contract）

1. 存储格式：PhotoIndex 中的 path 字段必须以标准的 UTF-8 字符串形式存储。
2. 语义边界：数据库记录的路径仅保证在【同一平台 + 同一文件系统语义】下有效。
3. 迁移限制：photo_core 不保证数据库在不同操作系统（如从 Windows 迁移到 macOS）之间的路径可用性。
4. 转换规则：进入数据库前的 path 必须经过安全检查，禁止 lossy 转换；无法转换为 UTF-8 的路径应在输入边界被拒绝。