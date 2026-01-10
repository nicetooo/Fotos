# EXIF 方向修复

## 问题描述
许多缩略图被旋转了 90 度，这是因为原始实现没有处理图片的 EXIF Orientation 标签。

## 解决方案

### 修改内容
在 `packages/core/src/image/thumbnail.rs` 中添加了 EXIF 方向处理：

1. **`read_exif_orientation()`** - 从图片文件读取 EXIF Orientation 标签
2. **`apply_orientation_correction()`** - 对图片字节应用方向校正
3. **`apply_orientation_to_image()`** - 对 DynamicImage 应用方向变换

### EXIF Orientation 值说明
- 1: 正常（无需旋转）
- 2: 水平翻转
- 3: 旋转 180°
- 4: 垂直翻转
- 5: 旋转 90° 顺时针 + 水平翻转
- 6: 旋转 90° 顺时针
- 7: 旋转 270° 顺时针 + 水平翻转
- 8: 旋转 270° 顺时针

### 处理流程
1. 读取源图片的 EXIF Orientation 标签
2. 如果使用嵌入的 EXIF 缩略图：
   - 解码嵌入的缩略图
   - 应用方向校正
   - 重新编码为 JPEG
3. 如果使用完整图片解码：
   - 解码完整图片
   - 应用方向校正
   - 生成缩略图

## 清除旧缓存

由于之前生成的缩略图没有应用方向校正，你需要清除缩略图缓存：

### macOS
```bash
rm -rf ~/Library/Application\ Support/com.fotos.app/thumbnails
```

或者在应用的 Settings 页面点击 "Clear Thumbnail Cache" 按钮。

## 测试
重新导入照片或清除缓存后，所有缩略图应该显示正确的方向。
