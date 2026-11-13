# 足迹相册 (footos) - 开发指南

跨平台照片管理应用：地图可视化 + Desktop/iOS/Android

## 架构分层

```
packages/
├── core/              # Rust 核心引擎（纯逻辑，零平台依赖）
├── desktop/           # Tauri 应用，三端共用（Svelte 前端 + Rust 后端 + gen/ 下的 iOS/Android 原生工程）
├── shared/            # 跨平台抽象层（traits + 类型定义）— 当前未被应用依赖
└── platform-{ios,android}/  # 上述 traits 的 Rust 实现 — 当前未被应用依赖
```

实际运行路径（Tauri 应用的 `Cargo.toml` 只依赖 `footos-core`）：

```
原生桥（iOS: PhotoLibraryBridge.m / Android: bridge/*.kt）
  ⇄ WebView JS（src/lib/platform/ios.ts、android.ts）
  ⇄ Tauri 命令（src-tauri/src/lib.rs）
  ⇄ footos-core
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
│   ├── types.ts
│   ├── utils/
│   └── i18n/
├── desktop/             # 桌面端完整 UI
│   ├── App.svelte
│   └── components/
└── mobile/              # 移动端完整 UI
    ├── App.svelte
    └── components/
```

**隔离原则**：
- UI 组件按平台完全独立，不共享 .svelte 文件
- 仅 `shared/` 下的纯逻辑（utils、i18n、类型）可复用
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

## 应用标识

| 平台 | 标识符 | 说明 |
|------|--------|------|
| Android | `app.footos` | package name（`build.gradle.kts` 中的 `applicationId`） |
| iOS | `app.footos` | Bundle Identifier |
| Tauri | `app.footos` | `tauri.conf.json` 中的 `identifier` |

⚠️ 包名是 **`app.footos`**，所有 `adb` 命令必须使用此包名。

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
cd packages/desktop && pnpm test          # 前端测试（vitest）
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

# 方法2：杀掉 footos 相关进程
pkill -f "target/debug/footos"

# 确认端口已释放
lsof -i:1420 || echo "Port 1420 is free"
```

### 常见问题

- **Port 1420 is already in use**：先用上面的命令杀掉占用进程
- **beforeDevCommand terminated**：通常是 Vite 启动失败，检查端口或依赖

## 缩略图规格

- **唯一真相源**：`core/src/config.rs` 中的 `DEFAULT_THUMBNAIL_SIZE`（当前值 `256`）
- `ThumbnailSpec::default()` 自动使用该常量，所有调用方（Tauri `lib.rs`、`platform-ios`、`platform-android`）统一引用
- ❌ 禁止在任何 crate 中硬编码缩略图尺寸魔数（如 `ThumbnailSpec { width: 300, height: 300 }`）
- ⚠️ 修改 `DEFAULT_THUMBNAIL_SIZE` 会改变缓存 key（`thumbnail_key` 将 width/height 哈希进 FNV-1a），导致所有已有缩略图缓存失效，需配合缓存清除

## 数据库

SQLite 单表 `photos`，关键字段：`path`(唯一)、`hash`(感知哈希去重)、`lat/lon`(地理位置)、`date_taken`、`media_type`(photo / video / live_photo)、`video_path`(实况照片的配对视频)

iOS 相册同步的行不拷贝原片：`path` 为 `ios-photo://<PhotoKit localIdentifier>`，原片在查看时按需导出。

## 技术栈

- 前端：Svelte 5 + TypeScript + Tailwind + MapLibre GL
- 后端：Tauri 2 + Rust
- 移动端桥接：iOS Objective-C（`PhotoLibraryBridge.m`，WebKit MessageHandler + PhotoKit）/ Android Kotlin（`FootosWebViewBridge`，`@JavascriptInterface`）
- 视频：桌面端调用系统 `ffmpeg` / `ffprobe` CLI 生成缩略图和读取元数据

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

## 移动端性能红线

以下优化经实测验证（671 张照片，Pixel 7a），修改相关代码时必须保持这些约束。

### 地图渲染：视口查询 + GPU 图层，禁止 DOM Marker

- ❌ 禁止使用 `maplibregl.Marker` 渲染照片点（670+ DOM 节点会导致严重卡顿）
- ❌ 禁止把整个照片库传给地图组件或在前端聚类（成本随库规模线性增长）
- ✅ 地图只画 Rust 返回的结果：`query_map(bbox, zoom, start_ms, end_ms)` 在 `footos-core` 的 `GeoIndex` 里按世界对齐网格聚类，只返回（外扩 50% 的）视口内的聚类和单点
- ✅ 结果 `setData` 到**非聚类**的 GeoJSON Source，由 Symbol/Circle Layer 绘制；`count > 1` 为聚类
- 触发查询：`move`（节流 120ms）、`moveend`、时间过滤变化、`refreshKey` 变化（库有变更）
- 缩略图通过 `map.addImage()` 注入 GL atlas，由 `styleimagemissing` 事件按需加载。图片 ID 由缩略图路径决定（`thumb:<path>` / `cthumb:<path>`），重新查询后不会失效
- 图层 ID：`CLUSTER_THUMB_LAYER_ID`（聚类缩略图）、`UNCLUSTERED_THUMB_LAYER_ID`（单点缩略图）

### 聚类点击定位：fitBounds 到成员包围盒

- ✅ 每个聚类要素带有成员的包围盒（属性 `w/s/e/n`，Rust 计算），点击时直接 `fitBounds`
- ✅ zoom ≥ 17 或包围盒退化为一个点时（同一地点的多张照片），点击直接打开查看器
- ❌ 禁止用"放大固定级数"之类的方式定位（可能放大后聚类仍不分裂）
- 点击处理使用 `queryRenderedFeatures(bbox)` + 取离触摸点最近的要素（bbox 内可能有多个聚类）

### GL 内存控制：瓦片 + 帧缓冲 + 缩略图

实测 GL mtrack 从 228MB 降至 94MB（交互后），总 PSS 从 490MB 降至 346MB。

- **@1x 瓦片**：使用 `{z}/{x}/{y}.png` 而非 `@2x.png`。每张瓦片 256×256×4=256KB（@2x 为 1MB）
- **底图来源**：OpenStreetMap 标准瓦片，定义在 `shared/map-style.ts`（桌面、移动共用）。只有浅色一种样式，深色主题靠 raster paint 压暗；CARTO 的瓦片现在需要 API key，不再使用
- ❌ 禁止在移动端使用 `@2x` 瓦片（GL 内存会翻 4 倍）
- **瓦片缓存上限**：Map 构造器设置 `maxTileCacheSize: 50`，限制 GPU 瓦片缓存
- **帧缓冲降分辨率**：Map 构造器设置 `pixelRatio: 2`（设备默认 DPR=2.75），降低 WebGL canvas 像素数
- **缩略图 canvas**：`THUMB_CANVAS_SIZE=66`、`THUMB_PIXEL_RATIO=1.5`（显示 44px），每张 66×66×4=17KB
- ❌ 禁止将 `THUMB_CANVAS_SIZE` 恢复到 88 或 `pixelRatio` 恢复到 2（GPU 内存回涨）

### 缩略图内存管理：LRU + Atlas 淘汰

- **JS 堆**：`thumbDataCache`（LRU Map，最大 120 条，~2MB）。`thumbCacheGet()` 访问时移到末尾，`thumbCacheSet()` 超限时淘汰最旧
- **GPU Atlas**：最大 180 张图。`evictUnusedImages()` 在 `idle` 事件后执行，`removeImage()` 当前查询结果里没有用到的图。被淘汰的图会由 `styleimagemissing` 自动重新加载
- ❌ 禁止无限制调用 `map.addImage()` 而不淘汰（GPU 内存会持续增长）

### 地图↔时间轴同步：单向数据流，没有循环

数据只沿两条互不相交的路径流动，所以不需要 `syncSource` 之类的防循环标记：

| 路径 | 触发 | 做什么 | 延迟 |
|------|------|--------|------|
| 地图 → 时间轴 | `moveend` | `query_timeline(视口 bbox)` 取可见照片的时间戳，交给时间轴显示 | 300ms 防抖 |
| 时间轴 → 地图 | **用户**操作时间轴 | `onTimeRangeChange(startMs, endMs)` → 设置时间过滤 → `query_map` | 120ms 防抖 |

- ❌ 地图移动**永远不**写时间过滤；时间轴的数据变化（`stamps` 换了）**永远不**触发 `onTimeRangeChange`
- 时间轴内部用 `userActive` 区分：只有用户手势会把它置为 true；选区重新覆盖全部范围时置回 false 并上报 `(null, null)` 清除过滤
- 用户设置了时间范围后，地图移动不再替换时间轴的数据（否则会重置用户刚选的范围）；清除过滤后恢复跟随地图
- 视口内没有照片时保留上一份时间轴数据，不清空
- 时间统一为"墙上时钟毫秒"（`parse_date_ms`，不做时区换算），显示时用 `timeZone: 'UTC'` 格式化

**主题切换后的样式恢复**：

`map.once('style.load')` 在 Android WebView 中对内联样式不可靠，使用 `requestAnimationFrame` + `map.isStyleLoaded()` 轮询代替。切换前 `map.off()` 移除所有事件监听，样式就绪后重新注册。

### TimelineSlider 计算优化

- 输入是排好序的 `Float64Array` 时间戳（`stamps`），不是照片数组
- 窗口内数量、左右两侧数量、边缘堆叠线数、每个像素列是否有照片，全部用 `shared/utils/timeline-index.ts` 里的二分查找计算，成本 O(像素宽度 · log n)
- 概览轴只在 `stamps` 或尺寸变化时重画，拖动黄框/精细轴时不重画
- canvas 位图尺寸只在实际变化时设置；尺寸由 `ResizeObserver` 维护，绘制路径里不调用 `getBoundingClientRect()`
- 精细轴的点阵背景是 CSS background，不在 canvas 里画
- ❌ 禁止在 touchmove 路径上遍历或排序全部时间戳
- ❌ 禁止在前端解析日期字符串来做时间轴计算（Rust 侧已解析）

### 按需加载（前端不持有照片库）

- 移动端 `App.svelte` 不保存照片列表。地图、时间轴、查看器各自向 Rust 查询当前需要的那一部分
- 查看器：`query_album(center_id, radius)` 取时间顺序相册里当前照片前后各 300 张的窗口；浏览到距边缘 60 张以内时，停顿 500ms 后以当前照片为中心重新取
- 库发生变化（导入进度、删除、清空）时只递增 `refreshKey`，由地图重新查询；Rust 侧 `AppState::geo()` 按 DB 变更计数惰性重建内存索引
- 只在照片**第一次出现**时自动 `fitBounds`；后续导入不得抢镜头
- ❌ 禁止通过 invoke 返回全量照片对象（iOS WKWebView 上 5k+ 对象约 8s）。需要批量数值时用 `tauri::ipc::Response` 返回原始字节
- 桌面端的图库抽屉仍然用 `list_photos` 全量加载，桌面端时间轴仍是旧实现（尚未迁移）

### 精细时间轴边缘堆叠

当照片在精细时间轴视图范围之外时，在边缘显示堆叠的竖线提示还有更多照片。

**核心规则**：
1. **紧密堆叠，无间隙**：所有竖线紧密排列，每条线相邻，中间不留空白
2. **重叠保持**：时间差小于 1 像素时间量（viewDurationMs / 350）的照片视为重叠，只显示一条线
3. **线条数 = 唯一时间位置数**：基于时间间隔在数据层计算，避免渲染时跳变
4. **最大宽度限制**：堆叠区域最大为屏幕宽度的 20%
5. **固定边缘**：左边堆叠从 x=0 开始向右画，右边堆叠从 x=width 开始向左画
6. **显示总数**：标签显示照片总数（不是唯一位置数）
