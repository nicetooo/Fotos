# 足迹相册 (footos)

一个现代化的照片管理桌面应用，专注于地理位置可视化和 RAW 文件支持。

## 功能特性

### 照片导入与管理

- **文件夹/文件导入** - 支持递归扫描文件夹或选择单个文件导入
- **多格式支持** - JPEG、PNG、WebP 以及多种 RAW 格式
- **实时进度** - 导入过程中显示实时进度和状态
- **智能去重** - 基于感知哈希的照片去重，避免重复导入
- **SQLite 索引** - 快速的数据库查询和照片检索

### RAW 文件支持

- **广泛的格式支持** - CR2、CR3、NEF、NRW、ARW、SRF、SR2、DNG、RAF、ORF、RW2、PEF、RAW
- **RAW+JPEG 配对** - 自动识别同名的 RAW 和 JPEG 文件并分组显示
- **缩略图生成** - 提取 EXIF 嵌入缩略图用于网格显示
- **方向校正** - 自动应用 EXIF 方向信息

> **注意**: RAW 文件的全尺寸预览提取目前仅支持部分相机格式。较新的相机（如 Nikon Zf）使用的 NEF 格式暂不支持全尺寸预览提取，仅显示小缩略图。

### 地图与地理位置

- **全屏地图界面** - 基于 Leaflet.js 的交互式地图作为主界面
- **照片标记** - 在地图上以圆形缩略图显示有 GPS 信息的照片
- **悬停预览** - 鼠标悬停时显示大图预览和元数据
- **智能聚焦** - 自动缩放地图以适应所有标记点

### 时间轴筛选

- **双层时间轴** - 总览轨道 + 缩放窗口的双层设计
- **范围选择** - 拖动手柄调整时间范围
- **窗口模式** - 支持 1小时、6小时、1天、7天、30天等固定窗口
- **密度可视化** - 显示照片在时间上的分布密度
- **实时过滤** - 根据时间范围实时过滤地图标记

### 照片预览

- **全屏预览** - 深色背景的沉浸式预览体验
- **缩放支持** - Ctrl/Cmd + 滚轮缩放，双击重置
- **键盘导航** - 方向键切换照片
- **元数据面板** - 显示拍摄日期、尺寸、相机、曝光参数、GPS 坐标等
- **快速定位** - 一键在 Finder 中显示文件位置

### 图库抽屉

- **虚拟滚动** - 基于 TanStack Virtual 的高性能网格
- **排序选项** - 按日期、名称、尺寸排序
- **懒加载** - 使用 IntersectionObserver 实现图片懒加载
- **RAW 标记** - 显示 RAW 文件和 RAW+JPEG 配对状态

### 主题系统

- **三种主题** - 深色、浅色、跟随系统
- **实时切换** - 主题切换即时生效
- **持久化** - 主题偏好保存到本地存储
- **系统监听** - 自动响应系统主题变化

### 快捷键

| 快捷键 | 功能 |
|--------|------|
| `Escape` | 关闭预览/设置/抽屉 |
| `←` / `→` | 预览中切换照片 |
| `Cmd/Ctrl + I` | 导入文件夹 |
| `+` / `-` | 预览中缩放 |
| `0` | 重置缩放 |

## 技术栈

### 前端
- **Svelte 5** - 响应式 UI 框架
- **Tailwind CSS** - 原子化 CSS 框架
- **TypeScript** - 类型安全
- **Vite** - 快速构建工具
- **Leaflet.js** - 地图库
- **TanStack Virtual** - 虚拟滚动

### 后端
- **Tauri 2** - 跨平台桌面框架
- **Rust** - 高性能核心引擎
- **SQLite** - 本地数据库
- **UniFFI** - 跨平台库生成

### 核心依赖
- `image` - 图像处理
- `kamadak-exif` - EXIF 解析
- `image_hasher` - 感知哈希
- `walkdir` - 目录遍历
- `rusqlite` - SQLite 绑定

## 项目结构

```
packages/
├── core/           # Rust 核心库
│   └── src/
│       ├── fs/     # 文件扫描
│       ├── image/  # 图像处理、缩略图、哈希
│       ├── index/  # 数据库索引
│       └── types.rs
├── desktop/        # Tauri 桌面应用
│   ├── src/        # Svelte 前端
│   │   ├── components/
│   │   │   ├── Map.svelte
│   │   │   ├── TimelineSlider.svelte
│   │   │   ├── ImagePreview.svelte
│   │   │   ├── Settings.svelte
│   │   │   └── VirtualPhotoGrid.svelte
│   │   ├── App.svelte
│   │   └── types.ts
│   └── src-tauri/  # Tauri/Rust 后端
```

## 开发

```bash
# 安装依赖
cd packages/desktop
npm install

# 开发模式
npm run tauri dev

# 构建
npm run tauri build
```

## 数据存储

应用数据存储在系统 Application Support 目录：

- `footos.db` - SQLite 数据库
- `thumbnails/` - 缩略图缓存
- `raw_previews/` - RAW 预览缓存
- `tiles/` - 地图瓦片缓存

## 许可证

MIT
