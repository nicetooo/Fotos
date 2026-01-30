/**
 * Dead code detection tests for Phase 1.1 and 1.2
 *
 * These tests scan active source code to prove that certain files/modules
 * are never imported, making them safe to delete.
 */
import { describe, it, expect } from "vitest";
import * as fs from "node:fs";
import * as path from "node:path";

const SRC_DIR = path.resolve(__dirname, "..");

/**
 * Recursively collect all files matching given extensions under a directory.
 */
function collectFiles(dir: string, exts: string[]): string[] {
  const results: string[] = [];
  if (!fs.existsSync(dir)) return results;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      // Skip node_modules and __tests__
      if (entry.name === "node_modules" || entry.name === "__tests__") continue;
      results.push(...collectFiles(fullPath, exts));
    } else if (exts.some((ext) => entry.name.endsWith(ext))) {
      results.push(fullPath);
    }
  }
  return results;
}

/**
 * Active code directories: desktop/, mobile/, shared/, lib/platform/
 * These are reachable from main.ts.
 */
const ACTIVE_DIRS = [
  path.join(SRC_DIR, "desktop"),
  path.join(SRC_DIR, "mobile"),
  path.join(SRC_DIR, "shared"),
  path.join(SRC_DIR, "lib", "platform"),
];

function getActiveFiles(): string[] {
  const mainTs = path.join(SRC_DIR, "main.ts");
  const files = [mainTs];
  for (const dir of ACTIVE_DIRS) {
    files.push(...collectFiles(dir, [".ts", ".svelte", ".js"]));
  }
  return files;
}

function readFileContent(filePath: string): string {
  return fs.readFileSync(filePath, "utf-8");
}

describe("Phase 1.1: src/lib/utils.ts is dead code", () => {
  it("no active file imports from lib/utils", () => {
    const allSourceFiles = collectFiles(SRC_DIR, [".ts", ".svelte", ".js"]);
    const importPatterns = [
      /from\s+['"].*lib\/utils['"]/,
      /import\s+.*['"].*lib\/utils['"]/,
    ];

    const violations: string[] = [];
    for (const file of allSourceFiles) {
      // Skip the test file itself
      if (file.includes("__tests__")) continue;
      const content = readFileContent(file);
      for (const pattern of importPatterns) {
        if (pattern.test(content)) {
          violations.push(`${path.relative(SRC_DIR, file)} matches ${pattern}`);
        }
      }
    }

    expect(violations).toEqual([]);
  });
});

describe("Phase 1.2: legacy frontend code is dead", () => {
  it("main.ts only imports desktop/App.svelte and mobile/App.svelte", () => {
    const mainTs = path.join(SRC_DIR, "main.ts");
    const content = readFileContent(mainTs);

    // Should import desktop and mobile App
    expect(content).toContain("./desktop/App.svelte");
    expect(content).toContain("./mobile/App.svelte");

    // Should NOT import root App.svelte
    const rootAppImport = /import\s+.*from\s+['"]\.\/App\.svelte['"]/;
    expect(rootAppImport.test(content)).toBe(false);

    // Should NOT have dynamic import of root App.svelte
    const dynamicRootApp = /import\(['"]\.\/App\.svelte['"]\)/;
    expect(dynamicRootApp.test(content)).toBe(false);
  });

  it("no active code imports from src/components/", () => {
    const activeFiles = getActiveFiles();
    const violations: string[] = [];

    for (const file of activeFiles) {
      const content = readFileContent(file);
      // Match imports that resolve to the root src/components/ directory
      // e.g., from "../components/..." or from "../../components/..."
      // But NOT from "./components/..." (which is relative to desktop/ or mobile/)
      const relPath = path.relative(SRC_DIR, file);
      const isRootLevel = !relPath.startsWith("desktop/") &&
                          !relPath.startsWith("mobile/") &&
                          !relPath.startsWith("shared/") &&
                          !relPath.startsWith("lib/");

      if (isRootLevel) {
        // For root-level files (like main.ts), check for ./components/ imports
        if (/from\s+['"]\.\/components\//.test(content)) {
          violations.push(`${relPath} imports from ./components/`);
        }
      } else {
        // For nested files, check they don't reach up to src/components/
        // A file in desktop/ importing ../../components/ would reach src/components/
        // But ./components/ means desktop/components/ which is fine
        const depth = relPath.split("/").length - 1;
        const upPattern = "../".repeat(depth) + "components/";
        if (content.includes(upPattern) && depth > 0) {
          violations.push(`${relPath} imports from root components/ via ${upPattern}`);
        }
      }
    }

    expect(violations).toEqual([]);
  });

  it("no active code imports from root src/App.svelte", () => {
    const activeFiles = getActiveFiles();
    const violations: string[] = [];

    for (const file of activeFiles) {
      const content = readFileContent(file);
      const relPath = path.relative(SRC_DIR, file);

      // Skip main.ts (checked separately above)
      if (relPath === "main.ts") continue;

      // Check for imports that would resolve to src/App.svelte
      if (/from\s+['"]\.\.\/App\.svelte['"]/.test(content) ||
          /from\s+['"]\.\.\/\.\.\/App\.svelte['"]/.test(content)) {
        violations.push(`${relPath} imports root App.svelte`);
      }
    }

    expect(violations).toEqual([]);
  });

  it("no active code imports from root src/types.ts", () => {
    const activeFiles = getActiveFiles();
    const violations: string[] = [];

    for (const file of activeFiles) {
      const content = readFileContent(file);
      const relPath = path.relative(SRC_DIR, file);

      // We need to check if an import resolves to src/types.ts (dead) vs src/shared/types.ts (active)
      // The key insight: only a file directly in src/ (depth 0) importing "./types" would hit src/types.ts
      // Or a file in src/desktop/ importing "../types", or src/desktop/components/ importing "../../types"
      const fileDir = path.dirname(file);
      const importPatterns = content.match(/from\s+['"]([^'"]+types)['"]/g) || [];

      for (const imp of importPatterns) {
        const specMatch = imp.match(/from\s+['"]([^'"]+)['"]/);
        if (!specMatch) continue;
        const specifier = specMatch[1];

        // Only check relative imports
        if (!specifier.startsWith(".")) continue;

        // Resolve the import to an absolute path
        const resolved = path.resolve(fileDir, specifier);
        const resolvedRel = path.relative(SRC_DIR, resolved);

        // If it resolves to "types" (i.e., src/types.ts), that's the dead root file
        if (resolvedRel === "types") {
          violations.push(`${relPath} imports root types.ts via "${specifier}"`);
        }
      }
    }

    expect(violations).toEqual([]);
  });

  it("no active code imports @tanstack/svelte-virtual", () => {
    const activeFiles = getActiveFiles();
    const violations: string[] = [];

    for (const file of activeFiles) {
      const content = readFileContent(file);
      if (content.includes("@tanstack/svelte-virtual")) {
        violations.push(path.relative(SRC_DIR, file));
      }
    }

    expect(violations).toEqual([]);
  });
});
