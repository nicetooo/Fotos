# Fotos Core

Rust 核心引擎库，负责跨平台的底层逻辑。

## 角色与定位
- 纯 Rust 引擎库，平台无关。
- 可被桌面 / 移动端 / WebView 外壳复用。

## 允许的职责
- 文件遍历 (PathBuf)
- 图片解码 / 转码 (JPEG / PNG / HEIC / AVIF)
- EXIF / 元数据解析
- 缩略图生成
- Hash / 感知 Hash / 去重
- SQLite 索引与查询
- 缓存 / 批处理 / 算法策略

## 绝对禁止
- ❌ 平台 API (AppKit / Win32 / Android SDK 等)
- ❌ 平台判断 (cfg!(target_os) 业务分支)
- ❌ UI / 交互逻辑
- ❌ 权限管理
- ❌ async/await
- ❌ 线程池 / 复杂并发
- ❌ 全局状态 / 单例
- ❌ 回调 / 闭包作为 API 输入
- ❌ 返回大图内存 / Base64

## 输入 / 输出边界
- 输入：PathBuf, Vec<u8>, String, 纯数据 Config / ID
- 输出：Result<T, CoreError>, PathBuf, 纯数据结构
