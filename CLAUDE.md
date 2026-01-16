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
pnpm desktop                              # 启动桌面开发（从项目根目录）
cd packages/core && cargo test            # Rust 测试
cd packages/desktop && pnpm tauri build   # 构建发布

# iOS 开发（模拟器，默认 iPhone 14 Pro）
cd packages/desktop
pnpm tauri ios dev "iPhone 14 Pro"

# Android 打包（必须用 tauri 命令，assets 嵌入在 Rust native library 里）
cd packages/desktop
pnpm tauri android build --debug --target aarch64
/opt/homebrew/share/android-commandlinetools/platform-tools/adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

## 开发服务器

### 桌面开发启动

```bash
pnpm desktop                              # 从项目根目录启动
```

- Vite 开发服务器端口：**1420**（strictPort，必须可用）
- HMR 端口：**1421**（可选，设置 TAURI_DEV_HOST 时启用）
- 修改 Svelte 文件会自动热更新，无需手动重启

### 重启前清理

如果端口被占用或需要重启：

```bash
# 方法1：杀掉占用端口的进程
lsof -ti:1420 | xargs kill -9

# 方法2：杀掉 fotos 相关进程
pkill -f "target/debug/fotos"

# 确认端口已释放
lsof -i:1420 || echo "Port 1420 is free"
```

### 常见问题

- **Port 1420 is already in use**：先用上面的命令杀掉占用进程
- **beforeDevCommand terminated**：通常是 Vite 启动失败，检查端口或依赖

## 数据库

SQLite 单表 `photos`，关键字段：`path`(唯一)、`hash`(感知哈希去重)、`lat/lon`(地理位置)、`date_taken`

## 技术栈

- 前端：Svelte 5 + TypeScript + Tailwind + MapLibre GL
- 后端：Tauri 2 + Rust
- 移动端桥接：iOS Swift (WebKit MessageHandler) / Android JNI

## 移动端时间轴交互设计

### 布局（从上到下）
1. **总时间轴（Overview）** - 显示完整时间范围
2. **精细时间轴（Detail）** - 显示当前选中区间的放大视图

### 总时间轴
- 显示完整时间范围内所有照片的分布
- **黄色高亮框**：
  - **点击跳转**：点击框外区域，黄框跳转到该位置
  - **拖动移动**：可以拖动黄框左右移动
  - 框的大小由精细时间轴的缩放决定（只读，不可直接调节大小）
- **5种时间区间按钮**：1h, 6h, 1d, 7d, 30d
  - 选中的档位会高亮
  - 如果用户通过双指缩放调节过，显示当前实际的单位时间（如 "2h" "15min"）

### 精细时间轴
- **固定黄色边框**：外框不可调节大小
- **单指左右滑动**：滚动时间轴，浏览不同时间段
- **双指缩放**（任意比例）：
  - 张开 → 显示更短的时间范围（更细）
  - 捏合 → 显示更长的时间范围（更粗）
  - **智能步进**：根据当前时间范围级别调整步进单位
    - 天级范围 → 按天调整
    - 小时级范围 → 按小时调整
    - 分钟级范围 → 按分钟调整

### 交互总结
| 操作 | 位置 | 效果 |
|------|------|------|
| 点击 | 总时间轴（框外） | 黄框跳转到点击位置 |
| 拖动 | 总时间轴（黄框） | 移动黄框位置 |
| 点击按钮 | 时间区间按钮 | 设置精细时间轴范围 |
| 单指滑动 | 精细时间轴 | 滚动时间 |
| 双指缩放 | 精细时间轴 | 调节时间范围（智能步进） |

### 精细时间轴边缘堆叠

当照片在精细时间轴视图范围之外时，在边缘显示堆叠的竖线提示还有更多照片。

**核心规则**：
1. **紧密堆叠，无间隙**：所有竖线紧密排列，每条线相邻，中间不留空白
2. **重叠保持**：在当前缩放级别下，同一像素位置的多张照片只显示为一条线
3. **线条数 = 唯一像素位置数**：基于当前缩放级别计算像素位置，然后去重
4. **最大宽度限制**：堆叠区域最大 100px
5. **固定边缘**：左边堆叠从 x=0 开始向右画，右边堆叠从 x=width 开始向左画
6. **显示总数**：标签显示照片总数（不是唯一位置数）
