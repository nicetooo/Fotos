# 足迹相册 (footos) 全面代码审查报告

## 高优先级 — 性能与正确性问题

| # | 问题 | 位置 | 影响 |
|---|------|------|------|
| 1 | ~~**SQLite 无事务批量导入**~~ | `src-tauri/src/lib.rs` `import_photos` | ✅ 已修复：PhotoIndex 新增 `begin/commit/rollback_transaction()` API，所有批量导入/更新/删除循环已包裹事务（100 条批量提交） |
| 2 | ~~**每次调用重新打开 DB 连接**~~ | `list_photos` / `list_photos_preview` 每次 `PhotoIndex::open()` | ✅ 已修复：引入 `AppState` 作为 Tauri Managed State，持有单一 `Arc<PhotoIndex>`。前端启动时调用 `init_app` 一次，11 个命令改用 `State<AppState>`，不再传递 `db_path`/`thumb_dir` |
| 3 | ~~**无 SQLite PRAGMA 优化**~~ | `photo_index.rs` | ✅ 已修复：连接打开时设置 `journal_mode=WAL`、`synchronous=NORMAL`、`cache_size=-8000`、`temp_store=MEMORY` |
| 4 | ~~**桌面端地图使用 DOM Marker**~~ | `desktop/components/Map.svelte` | ✅ 已修复：已迁移至 GeoJSON Source + GPU Clustering 方案，与移动端统一 |
| 5 | **桌面端照片列表无虚拟滚动** | `desktop/App.svelte` 的 Library Drawer | 全量渲染 CSS Grid，万张照片会卡死浏览器。旧版 `VirtualPhotoGrid` 有虚拟化但未被使用 |
| 6 | ~~**缩略图规格不一致**~~ | Tauri / Platform Service | ✅ 已修复：全部统一使用 `ThumbnailSpec::default()`，读取 `config.rs` 中的 `DEFAULT_THUMBNAIL_SIZE`（256） |

## 中优先级 — 架构与代码质量

| # | 问题 | 详情 |
|---|------|------|
| 7 | **平台服务层是死代码** | `footos-shared` + `footos-platform-desktop` 定义了完整的 trait 体系，但 Tauri `lib.rs` 直接调用 `footos-core`，完全绕过了平台抽象层 |
| 8 | **~2500 行遗留代码未清理** | `src/App.svelte`（1300+ 行）+ `src/components/`（6 个文件）完全不被 `main.ts` 引用，是死代码 |
| 9 | **30+ 处 `println!` 调试输出** | `lib.rs`、`photo_index.rs` 遍布 `println!`，应替换为 `log`/`tracing` 框架，支持按级别过滤 |
| 10 | **JPEG 解析逻辑重复** | `hash.rs` 和 `thumbnail.rs` 各自实现了 `find_jpeg_tiff_header_offset`；`is_heic_file`/`is_raw_file` 也有重复 |
| 11 | **`blake3` 依赖未使用** | `Cargo.toml` 声明了 `blake3 = "1.5"` 但无代码引用，属于无用依赖 |
| 12 | **`@tanstack/svelte-virtual` 未使用** | `package.json` 声明但只有遗留组件用它，活跃代码未使用 |
| 13 | **`to_string_lossy()` 路径转换** | 违反 CLAUDE.md 中"路径必须安全检查，禁止 lossy 转换"的约束 |
| 14 | **函数命名误导** | `platform-android` 中 `md5_hash()` 实际是简单滚动哈希，不是 MD5 |

## 功能增强建议

| # | 建议 | 详情 |
|---|------|------|
| 15 | ~~**并行导入**~~ | ✅ 已实现：`import_photos` 重构为 3 阶段流水线。Phase 1（顺序）过滤已导入照片；Phase 2（rayon 并行）compute_hash + read_metadata + generate_thumbnail；Phase 3（顺序）去重 + 事务批量写入 DB。rayon 仅添加到 Tauri 层（Core 层保持无线程池） |
| 16 | **批量删除优化** | `delete_by_ids` 逐条 `DELETE`，应用 `WHERE id IN (...)` 批量操作 |
| 17 | **HEIC 感知哈希** | 目前 HEIC 用文件路径+大小哈希，同一照片不同文件名无法去重。可考虑集成 `libheif` 或调用系统解码 |
| 18 | **桌面端 locale 缺失** | `desktop/TimelineSlider.svelte` 硬编码 `'en-US'`。`Map.svelte` 已修复，通过 `MapView` → `App` 传递 `locale` prop |
| 19 | **"Map View" 未国际化** | 时间轴中 `isMapConstrained` 时显示硬编码英文 "Map View"，应使用 i18n key |
| 20 | **权限 UI 未实现** | 17 种语言都翻译了权限相关字符串（limitedAccessTitle 等 6 个 key），但无对应 UI |
| 21 | **移动端无删除功能** | 移动端预览只有基本信息，无删除按钮（可能是设计意图，但未明确记录） |
| 22 | **`src/lib/utils.ts` 空文件** | 预留的工具函数文件，内容为空 |
| 23 | **TypeScript 类型松散** | 移动端 `Map.svelte` 和 `App.svelte` 大量使用 `any[]`，应用 `PhotoInfo`/`PhotoPreview` 类型 |
| 24 | **`created_at`/`modified_at` 永远为 None** | `PhotoInfo` 有这些字段但 DB 无对应列，查询永不填充 |

## 总体评价

| 维度 | 评级 | 说明 |
|------|------|------|
| **架构设计** | ★★★★★ | 分层清晰、平台隔离彻底、约束明确 |
| **错误处理** | ★★★★ | 优雅降级，无 panic，EXIF 解析容错好 |
| **移动端性能** | ★★★★★ | GPU 渲染、LRU 淘汰、防抖同步、单次遍历计算，非常精细 |
| **桌面端性能** | ★★★★ | GeoJSON+GPU 渲染已统一（地图），Library Drawer 仍无虚拟滚动 |
| **数据库性能** | ★★★★ | ~~无事务、无 WAL、无连接复用~~ ✅ 已优化：事务批量提交 + WAL + PRAGMA 调优 + 连接复用 + rayon 并行导入 |
| **代码整洁度** | ★★★ | 有死代码、重复逻辑、println 日志 |
| **国际化** | ★★★★ | 17 种语言全覆盖，少数硬编码遗漏 |
| **测试覆盖** | ★★★ | Core 层测试充分，Platform 和前端无测试 |

## 建议优先行动

1. ~~**数据库优化**（#1-3）~~ ✅ 已完成 — 事务批量提交 + WAL/PRAGMA + 连接复用
1b. ~~**并行导入**（#15）~~ ✅ 已完成 — rayon 3 阶段流水线（过滤 → 并行处理 → 顺序写入）
2. **清理死代码**（#8, #11, #12, #22）— 减少维护负担和困惑
3. **桌面端性能对齐**（#5）— ~~#4 地图已迁移至 GeoJSON+GPU~~ ；Library Drawer 虚拟滚动待实现
4. ~~**统一缩略图规格**（#6）~~ ✅ 已完成 — 全部使用 `ThumbnailSpec::default()`
5. **日志框架替换**（#9）— 用 `tracing` 替换 `println!`
