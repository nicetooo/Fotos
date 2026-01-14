# Fotos - 开发指南

跨平台照片管理应用：地图可视化 + Desktop/iOS/Android

## 架构分层

```
packages/
├── core/              # Rust 核心引擎（纯逻辑，零平台依赖）
├── shared/            # 跨平台抽象层（traits + 类型定义）
├── platform-*/        # 平台实现（desktop/ios/android）
└── desktop/           # Tauri 应用（Svelte 前端 + Rust 后端）
```

## 分层底线

### Core 层禁止

- ❌ 任何平台 API（AppKit、UIKit、Win32、Android SDK）
- ❌ 平台条件编译（`#[cfg(target_os)]` 的业务分支）
- ❌ async/await、线程池、全局状态
- ❌ 回调/闭包作为 API 输入
- ❌ 返回大内存数据（完整图片、Base64）

Core 只接受：`PathBuf`、`String`、`Vec<u8>`、纯数据结构
Core 只返回：`Result<T, Error>`、`PathBuf`、纯数据结构

### 前端平台化原则

- **单点检测**：平台检测只在 App 入口执行一次，通过 Context 传递
- **能力驱动**：UI 判断基于「能力」而非「平台名」（检查 `capabilities.revealInFileManager` 而非 `platform === 'macos'`）
- **服务隔离**：平台特有逻辑（iOS 桥接、Android JNI）封装在独立 Service 中

### 前端平台隔离结构

```
src/lib/platform/
├── types.ts        # 统一接口定义（PlatformService）
├── context.ts      # Svelte Context，提供平台服务实例
├── desktop.ts      # 桌面实现：Tauri 文件对话框
├── ios.ts          # iOS 实现：WebKit 桥接、事件监听
└── android.ts      # Android 实现：预留
```

**隔离规则**：
- 每个平台文件实现相同的 `PlatformService` 接口
- 平台特有代码（webkit.messageHandlers、事件监听）只出现在对应平台文件中
- App.svelte 通过 Context 获取服务实例，不直接判断平台类型
- 新增平台只需添加新文件并注册到工厂函数

### 前端 UI 隔离

所有 UI 组件按平台完全分离，修改时互不影响：

```
src/
├── shared/              # 纯逻辑复用（无 UI）
│   ├── stores/
│   └── utils/
├── desktop/             # 桌面端完整 UI
│   ├── App.svelte
│   ├── components/
│   └── layouts/
└── mobile/              # 移动端完整 UI
    ├── App.svelte
    ├── components/
    └── layouts/
```

**隔离原则**：
- UI 组件按平台完全独立，不共享 .svelte 文件
- 仅 `shared/` 下的纯逻辑（stores、utils、类型）可复用
- 入口根据平台加载 `desktop/App.svelte` 或 `mobile/App.svelte`

**跨平台只共享**：
- 组件命名（如都叫 `ImagePreview.svelte`）
- Props 命名（如都用 `photo`、`onClose`）
- Theme 定义（CSS 变量、Tailwind 配置）

### 平台能力差异

| 能力 | Desktop | iOS | Android |
|------|:-------:|:---:|:-------:|
| 任意路径访问 | ✓ | ✗ | ✗ |
| 原生相册集成 | ✗ | ✓ | ✓ |
| 运行时权限 | ✗ | ✓ | ✓ |
| 文件管理器显示 | ✓ | ✗ | ✗ |

## 环境变量

```bash
export JAVA_HOME=/opt/homebrew/Cellar/openjdk@17/17.0.17/libexec/openjdk.jdk/Contents/Home
export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools
```

## 常用命令

```bash
pnpm install                              # 安装依赖
pnpm desktop                              # 启动桌面开发
cd packages/core && cargo test            # Rust 测试
cd packages/desktop && pnpm tauri build   # 构建发布

# Android 打包（必须用 tauri 命令，assets 嵌入在 Rust native library 里）
cd packages/desktop
pnpm tauri android build --debug --target aarch64
/opt/homebrew/share/android-commandlinetools/platform-tools/adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

## 数据库

SQLite 单表 `photos`，关键字段：`path`(唯一)、`hash`(感知哈希去重)、`lat/lon`(地理位置)、`date_taken`

## 技术栈

- 前端：Svelte 5 + TypeScript + Tailwind + MapLibre GL
- 后端：Tauri 2 + Rust
- 移动端桥接：iOS Swift (WebKit MessageHandler) / Android JNI
