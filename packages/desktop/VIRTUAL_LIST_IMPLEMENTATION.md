# 虚拟列表和图片懒加载实现总结

## 概述
为 Fotos 桌面应用的图片列表添加了虚拟列表和图片懒加载功能，以提升大量图片场景下的性能。

## 主要变更

### 1. 安装依赖
- 添加了 `@tanstack/svelte-virtual` 用于虚拟滚动

### 2. 新增文件

#### `/packages/desktop/src/types.ts`
- 定义了共享的 TypeScript 类型接口
- 包含 `PhotoId`, `PhotoMetadata`, `PhotoInfo` 等类型

#### `/packages/desktop/src/components/VirtualPhotoGrid.svelte`
- 新的虚拟网格组件，使用 `@tanstack/svelte-virtual` 实现
- 特性：
  - 动态计算列数（基于容器宽度）
  - 虚拟滚动，只渲染可见区域的行
  - 使用 ResizeObserver 响应容器尺寸变化
  - 支持 2 行的 overscan（预渲染）

### 3. 修改文件

#### `/packages/desktop/src/components/ThumbnailImage.svelte`
- 添加了 `lazy` 属性支持懒加载
- 使用 IntersectionObserver API 实现图片懒加载
- 特性：
  - 200px 的 rootMargin（提前加载）
  - 只在图片进入视口附近时才加载
  - 自动清理 observer 和 object URLs

#### `/packages/desktop/src/App.svelte`
- 导入并使用 `VirtualPhotoGrid` 组件
- 移除了原有的静态网格实现
- 导入共享类型定义

## 性能优化

### 虚拟列表优化
- **减少 DOM 节点**：只渲染可见的行，而不是所有照片
- **动态列数**：根据窗口宽度自动调整列数
- **Overscan**：预渲染 2 行以提供流畅的滚动体验

### 图片懒加载优化
- **延迟加载**：只加载可见或即将可见的图片
- **提前加载**：200px 的 rootMargin 确保滚动时图片已准备好
- **内存管理**：自动清理不再使用的 blob URLs

## 使用方式

虚拟网格组件接受以下 props：
```typescript
{
  photos: PhotoInfo[];           // 照片列表
  uniqueTs: number;             // 刷新键
  onPhotoClick: (photo) => void; // 点击回调
  onShowInFinder: (path, e) => void; // 显示在 Finder 回调
  formatDate: (date?) => string; // 日期格式化函数
}
```

ThumbnailImage 组件新增 `lazy` 属性：
```svelte
<ThumbnailImage
  path={photo.path}
  alt="Photo"
  className="..."
  lazy={true}  <!-- 启用懒加载 -->
/>
```

## 预期效果

1. **更快的初始渲染**：只渲染可见的照片
2. **更少的内存占用**：减少 DOM 节点和图片加载
3. **流畅的滚动**：虚拟滚动 + 预加载
4. **响应式布局**：自动适应窗口大小

## 注意事项

- 虚拟列表需要固定的行高（通过 `estimateSize` 计算）
- 懒加载依赖 IntersectionObserver API（现代浏览器都支持）
- 确保照片列表已排序后再传入 VirtualPhotoGrid
