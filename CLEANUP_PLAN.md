# 死代码清理计划

> 基于 REVIEW.md 审查结果 + 二次逐项验证，分阶段清理项目中的无用代码。
> 核心原则：**每次清理前必须先有测试覆盖，确保不影响任何现有功能。**

## 现状总览

| 类别 | 现有测试 | 死代码规模 |
|------|---------|-----------|
| Rust Core | 16 个单元测试（5 个文件） | blake3 依赖、JPEG 重复逻辑（3 个函数）、`created_at`/`modified_at` 死字段 |
| Rust Platform | 0 个测试 | `platform-desktop` 整个 crate 未使用；`footos-shared` 在 Tauri 中未使用 |
| 前端 | 0 个测试、无测试框架 | ~2500 行遗留代码（12 个死文件 + 2 个死目录） |
| NPM 依赖 | -- | `@tanstack/svelte-virtual` 仅被死代码引用 |
| Rust 依赖 | -- | `blake3` 声明但零引用 |
| 调试代码 | -- | 51 处 `println!`/`eprintln!`（lib.rs 37 + core 9 + platform-desktop 5） |

## 执行顺序与依赖关系

```
Phase 0 (基础设施)
  |
  +---> Phase 1 (零风险清理)
  |      +-- 1.1 空文件 utils.ts
  |      +-- 1.2 遗留前端代码（12 个文件 + 2 个目录）
  |      +-- 1.3 @tanstack/svelte-virtual 依赖
  |      +-- 1.4 blake3 Rust 依赖
  |
  +---> Phase 2 (Rust crate 清理)
  |      +-- 2.1 移除 Tauri -> platform-desktop 依赖
  |      +-- 2.2 移除 Tauri -> shared 依赖
  |      +-- 2.3 删除 platform-desktop 目录
  |
  +---> Phase 3 (Core 层代码质量)
  |      +-- 3.1 JPEG/TIFF 解析去重（3 个函数）
  |      +-- 3.2 移除 created_at/modified_at 死字段
  |      +-- 3.3 重命名 md5_hash
  |
  +---> Phase 4 (println! -> tracing)
         +-- 4.1 引入 tracing 依赖
         +-- 4.2 替换 Core 层 9 处
         +-- 4.3 替换 Tauri 层 37 处
```

## 预估清理量

| Phase | 删除/修改行数 | 移除文件数 | 风险等级 |
|-------|-------------|-----------|---------|
| Phase 1 | ~2500 行删除 + 2 个依赖 | 12 个文件 + 2 个目录 | 极低 |
| Phase 2 | ~800 行删除（platform-desktop crate） | 6 个文件 + 1 个目录 | 低 |
| Phase 3 | ~200 行重构 | 1 个新文件 + 修改 4 个 | 中 |
| Phase 4 | ~46 行替换 + 依赖添加 | 修改 4 个文件 | 低 |
| **合计** | **~3500+ 行** | **~20 个文件** | -- |

---

## Phase 0：搭建测试基础设施

> 所有后续清理的前提条件。

### 0.1 配置 Vitest 前端测试框架 -- DONE

**目标**：让前端拥有可运行的测试环境。

**操作**：
1. ~~安装依赖：`pnpm add -D vitest`（在 `packages/desktop/` 下）~~
2. ~~创建 `packages/desktop/vitest.config.ts`~~
3. ~~在 `packages/desktop/package.json` 添加 `"test": "vitest run"` script~~
4. ~~写一个 smoke test 确认框架工作（`src/__tests__/smoke.test.ts`）~~

**验证**：`pnpm test` 通过 -- vitest v4.0.18, 1 test passed

### 0.2 确认 Rust 现有单元测试基线 -- DONE

**目标**：建立基线，确保当前代码健康。

**操作**：
```bash
cd packages/core && cargo test
```

**结果**：14 passed, 2 failed（pre-existing）
- `test_generate_workflow` 和 `test_idempotency_and_cache_hit` 失败：
  `generate_image_file` 已改为仅提取 EXIF 嵌入缩略图，合成 PNG 无嵌入缩略图导致失败。
  这是代码逻辑变更后测试未同步更新的预存问题，与本次清理无关。
- 基线：14 passed + 2 pre-existing failures

---

## Phase 1：零风险清理（无代码依赖）

> 删除完全孤立的文件和依赖，不可能影响任何功能。

### 1.1 删除空文件 `src/lib/utils.ts` (Review #22) -- DONE

**前置测试**：`dead-code.test.ts` 中 "no active file imports from lib/utils" 通过
**操作**：已删除 `src/lib/utils.ts`
**验证**：`pnpm build` 成功

---

### 1.2 删除遗留前端代码 (Review #8) -- DONE

**现状**：
- `main.ts` 动态 import `./desktop/App.svelte` 或 `./mobile/App.svelte`
- **从不** import `./App.svelte`（旧版入口）
- 以下 12 个文件完全不被活跃代码引用：

| 文件 | 说明 |
|------|------|
| `src/App.svelte` | ~1211 行，旧版单体入口 |
| `src/types.ts` | 32 行，`shared/types.ts` 的子集，仅被死代码引用 |
| `src/assets/react.svg` | Vite 脚手架残留，全项目零引用 |
| `src/components/Settings.svelte` | 旧版设置面板 |
| `src/components/Map.svelte` | 旧版 DOM Marker 地图 |
| `src/components/TimelineSlider.svelte` | 旧版时间轴 |
| `src/components/MapView.svelte` | 旧版地图视图容器 |
| `src/components/ImagePreview.svelte` | 旧版图片预览 |
| `src/components/VirtualPhotoGrid.svelte` | 旧版虚拟网格（引用 @tanstack/svelte-virtual） |
| `src/components/ThumbnailImage.svelte` | 旧版缩略图组件 |
| `src/lib/utils.ts` | 空文件（Phase 1.1 已删除） |

同时删除 2 个空目录：
- `src/components/`（删除全部文件后为空）
- `src/assets/`（删除 react.svg 后为空）

**前置测试**：`dead-code.test.ts` 中 5 个测试全部通过（main.ts 路由、src/components/ 零引用、root App.svelte 零引用、root types.ts 零引用、@tanstack/svelte-virtual 零引用）
**操作**：已删除 12 个文件 + 2 个空目录（`src/components/`、`src/assets/`）
**验证**：`pnpm build` 成功 + `pnpm test` 7 passed

---

### 1.3 移除 `@tanstack/svelte-virtual` 依赖 (Review #12) -- DONE

**前置测试**：复用 1.2 的 `dead-code.test.ts` 中 "@tanstack/svelte-virtual 零引用" 测试
**操作**：已从 `package.json` 移除，`pnpm install` 确认
**验证**：`pnpm build` 成功

---

### 1.4 移除 `blake3` Rust 依赖 (Review #11) -- DONE

**前置测试**：全项目 `.rs` 文件零 `blake3` 引用
**操作**：已从 `packages/core/Cargo.toml` 移除 `blake3 = "1.5"`
**验证**：`cargo build` + `cargo test` 通过（14 passed + 2 pre-existing failures）

---

## Phase 2：Rust 依赖与 crate 清理

> 移除编译了但从未被调用的 crate 依赖。

### 2.1 从 Tauri 移除 `footos-platform-desktop` 依赖 (Review #7) -- DONE

**前置测试**：`src-tauri/src/` 下零 `footos_platform_desktop` 引用
**操作**：已从 `src-tauri/Cargo.toml` 移除依赖行
**验证**：`cargo build` 成功

---

### 2.2 从 Tauri 移除未使用的 `footos-shared` 依赖 (Review #7) -- DONE

**前置测试**：`src-tauri/src/` 下零 `footos_shared` 引用
**操作**：已从 `src-tauri/Cargo.toml` 移除依赖行（与 2.1 同步执行）
**注意**：`footos-shared` 仍被 `platform-android` 和 `platform-ios` 使用
**验证**：`cargo build` 成功

---

### 2.3 删除 `packages/platform-desktop/` 目录 -- DONE

**前置测试**：确认无任何 Cargo.toml 引用 `platform-desktop`
**操作**：已删除整个 `packages/platform-desktop/` 目录（5 个源文件 + Cargo.toml）
**验证**：`cargo build` 成功

---

## Phase 3：Core 层代码质量优化

> 需要更深入的测试保护，涉及重构现有功能代码。

### 3.1 消除 JPEG/TIFF 解析逻辑重复 (Review #10) -- DONE

**前置测试**：7 个新测试写入 `image/parse.rs`，全部通过
**操作**：
- 创建 `packages/core/src/image/parse.rs`，提取 `find_jpeg_tiff_header_offset`、`is_heic_file`、`is_tiff_based`、`is_raw_file`
- `hash.rs` 移除 3 个重复函数，改为 `use crate::image::parse::*`
- `thumbnail.rs` 移除 3 个重复函数 + 1 个死代码 `pub fn is_heic_file`，改为 `use crate::image::parse::*`
- `thumbnail.rs` 中 `find_jpeg_tiff_header_offset` 调用处添加 `.map_err()` 转换 `CoreError` -> `ThumbnailError`
- `image/mod.rs` 添加 `pub(crate) mod parse;` + `pub use parse::is_raw_file;`
**验证**：21 passed（14 原有 + 7 新增） + 2 pre-existing failures

---

### 3.2 移除 `created_at`/`modified_at` 死字段 (Review #24) -- DONE

**前置测试**：现有 21 个 Rust 测试 + `pnpm build` 作为回归保护
**操作**：
- 从 `types.rs` 的 `PhotoInfo` struct 移除 `created_at` 和 `modified_at` 字段
- 移除 `photo_index.rs` 中 3 处 `created_at: None, modified_at: None` 赋值
- 从 `src/shared/types.ts` 移除对应 TypeScript 字段
- `src/types.ts` 已在 Phase 1.2 删除
**验证**：`cargo test` 21 passed + `pnpm build` 成功

---

### 3.3 重命名 `md5_hash` -> `quick_hash` (Review #14) -- DONE

**前置测试**：3 个新测试（deterministic、different_inputs、empty_input）写入 `photo_service.rs`
**操作**：
- 重命名函数为 `quick_hash`，更新文档注释为准确描述
- 更新唯一调用处
**验证**：`cargo check` 编译通过

---

## Phase 4：println!/eprintln! -> tracing 替换 (Review #9)

> 生产代码共 51 处诊断输出需替换。其中 5 处 `eprintln!` 在 `platform-desktop`（Phase 2 已删除），
> 实际需手动替换 **46 处**。

### 4.1 引入 tracing 依赖 -- DONE

**目标**：为 Core 和 Tauri 层添加结构化日志支持。

**操作**：
1. `packages/core/Cargo.toml` 添加 `tracing = "0.1"`
2. `packages/desktop/src-tauri/Cargo.toml` 添加 `tracing = "0.1"` + `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`
3. Tauri `lib.rs` 的 `run()` 函数中配置 subscriber：
   ```rust
   tracing_subscriber::fmt()
       .with_env_filter(
           tracing_subscriber::EnvFilter::try_from_default_env()
               .unwrap_or_else(|_| "footos=info".into()),
       )
       .init();
   ```
   默认 `footos=info`，可通过 `RUST_LOG=footos=debug` 环境变量覆盖。

**验证**：`cargo build` + `cargo test` 通过

---

### 4.2 替换 Core 层 println!（9 处） -- DONE

**操作**：替换 `photo_index.rs`（4 处）、`lib.rs`（2 处）、`exif.rs`（3 处）
**验证**：21 passed + 2 pre-existing failures

---

### 4.3 替换 Tauri 层 println!（37 处） -- DONE

**操作**：替换全部 37 处 `println!`，语义映射：
- `tracing::error!` -- DB 打开失败、文件拷贝失败、hash/metadata/insert 错误、路径不存在
- `tracing::warn!` -- 重复照片跳过
- `tracing::info!` -- Phase 完成摘要、导入开始/结束/结果、照片计数、init_app
- `tracing::debug!` -- 路径检查、URI 解码、单文件处理、存储目录、backfill 计数
- `tracing::trace!` -- list_photos/list_photos_preview 入口

所有 tracing 调用使用结构化字段（`key = value`），而非格式化字符串拼接。

**验证**：
- `cargo build` 成功（零错误）
- `cargo test`（core）21 passed + 2 pre-existing failures
- `pnpm test` 7 passed
- `pnpm build` 成功
- 全 Rust 代码库零 `println!`/`eprintln!` 残留（grep 确认）

---

## 每阶段提交策略

| 完成节点 | Git Commit |
|----------|-----------|
| Phase 0 完成 | `chore: setup vitest and verify rust test baseline` |
| Phase 1 完成 | `chore: remove ~2500 lines dead frontend code and unused deps` |
| Phase 2 完成 | `chore: remove unused platform-desktop crate and stale cargo deps` |
| Phase 3 完成 | `refactor: deduplicate JPEG parsing, remove dead fields, fix naming` |
| Phase 4 完成 | `refactor: replace 46 println!/eprintln! with structured tracing` |

---

## 风险评估与回滚

| Phase | 风险 | 回滚方式 |
|-------|------|---------|
| Phase 1 | 极低 -- 删除的代码已证明无引用 | `git revert` 单次提交 |
| Phase 2 | 低 -- Cargo 依赖移除可能影响交叉编译 | `git revert` + 重新添加依赖 |
| Phase 3 | 中 -- 重构可能引入行为差异 | 前置测试覆盖 + `git revert` |
| Phase 4 | 低 -- 纯输出通道变更 | `git revert` |

---

## 不在清理范围内的项目

以下 REVIEW.md 中提及的问题**不包含在本次清理中**（需要功能开发或设计决策）：

| # | 问题 | 原因 |
|---|------|------|
| 5 | 桌面端虚拟滚动 | 功能增强，非死代码 |
| 13 | `to_string_lossy()` 路径转换 | 安全性改进，需要更细致的审查 |
| 16 | 批量删除 SQL 优化 | 性能优化，非死代码 |
| 17 | HEIC 感知哈希 | 功能增强 |
| 18-19 | 国际化遗漏 | 功能增强 |
| 20-21 | 权限 UI / 移动端删除 | 功能开发 |
| 23 | TypeScript 类型松散 | 代码质量，需逐步改进 |
