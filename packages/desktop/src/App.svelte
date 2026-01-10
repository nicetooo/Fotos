<script lang="ts">
    import { onMount } from "svelte";
    import { invoke } from "@tauri-apps/api/core";

    let version = $state("Loading...");
    let importStatus = $state({ success: 0, failure: 0 });
    let isScanning = $state(false);
    let error = $state("");

    onMount(async () => {
        try {
            version = await invoke("get_core_version");
        } catch (e) {
            error = "Failed to load core version";
        }
    });

    async function handleScan() {
        isScanning = true;
        error = "";
        try {
            const result = await invoke("import_photos", {
                rootPath: "/Users/nice/Pictures",
                dbPath: "fotos.db",
                thumbDir: "thumbnails",
            });
            importStatus = result as any;
        } catch (e) {
            error = String(e);
        } finally {
            isScanning = false;
        }
    }
</script>

<main
    class="flex h-screen w-full bg-[#0f172a] text-slate-200 overflow-hidden font-sans"
>
    <!-- Sidebar -->
    <aside
        class="w-64 border-r border-slate-800 bg-[#0f172a]/50 backdrop-blur-xl flex flex-col p-6 gap-8"
    >
        <div class="flex items-center gap-3 px-2">
            <div
                class="w-8 h-8 rounded-xl bg-gradient-to-tr from-indigo-500 to-purple-500 flex items-center justify-center shadow-lg shadow-indigo-500/20"
            >
                <i class="fa-solid fa-camera text-white text-sm"></i>
            </div>
            <h1 class="text-xl font-bold tracking-tight text-white">Fotos</h1>
        </div>

        <nav class="flex-1 space-y-2">
            <button
                class="w-full flex items-center gap-3 px-4 py-3 rounded-xl bg-white/5 text-white transition-all hover:bg-white/10 group"
            >
                <i
                    class="fa-solid fa-images text-indigo-400 group-hover:scale-110 transition-transform"
                ></i>
                <span class="font-medium">Library</span>
            </button>
            <button
                class="w-full flex items-center gap-3 px-4 py-3 rounded-xl text-slate-400 transition-all hover:bg-white/5 hover:text-white group"
            >
                <i
                    class="fa-solid fa-arrows-rotate group-hover:rotate-180 transition-transform duration-500"
                ></i>
                <span class="font-medium">Activities</span>
            </button>
            <button
                class="w-full flex items-center gap-3 px-4 py-3 rounded-xl text-slate-400 transition-all hover:bg-white/5 hover:text-white group"
            >
                <i
                    class="fa-solid fa-gear group-hover:rotate-45 transition-transform"
                ></i>
                <span class="font-medium">Settings</span>
            </button>
        </nav>

        <div
            class="mt-auto p-4 rounded-2xl bg-gradient-to-br from-indigo-500/10 to-purple-500/10 border border-indigo-500/20"
        >
            <div class="flex items-center gap-2 mb-2 text-indigo-300">
                <i class="fa-solid fa-circle-info text-[10px]"></i>
                <span class="text-xs font-semibold uppercase tracking-wider"
                    >Engine Info</span
                >
            </div>
            <p class="text-sm text-slate-400">
                Version: <span class="text-white font-mono">{version}</span>
            </p>
        </div>
    </aside>

    <!-- Main Content -->
    <section class="flex-1 flex flex-col p-8 overflow-y-auto relative">
        <!-- Header -->
        <header class="flex justify-between items-end mb-12">
            <div>
                <h2 class="text-3xl font-extrabold text-white mb-2">
                    Welcome Back
                </h2>
                <p class="text-slate-400">
                    Manage your memories with high-precision Rust engine.
                </p>
            </div>

            <button
                onclick={handleScan}
                disabled={isScanning}
                class="flex items-center gap-3 px-6 py-3 rounded-2xl bg-indigo-600 hover:bg-indigo-500 active:scale-95 text-white font-semibold transition-all shadow-lg shadow-indigo-600/20 disabled:opacity-50 disabled:cursor-not-allowed group"
            >
                <i
                    class="fa-solid fa-folder-open text-lg {isScanning
                        ? 'animate-pulse'
                        : 'group-hover:-rotate-12 transition-transform'}"
                ></i>
                <span>{isScanning ? "Scanning..." : "Import Photos"}</span>
            </button>
        </header>

        <!-- Dashboard Status -->
        <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
            <div
                class="p-6 rounded-3xl bg-slate-800/40 border border-slate-700/50 backdrop-blur-sm transition-all hover:border-indigo-500/30"
            >
                <div
                    class="text-slate-400 text-sm font-medium mb-1 uppercase tracking-tight"
                >
                    Photos Indexed
                </div>
                <div class="text-4xl font-black text-white">
                    {importStatus.success}
                </div>
            </div>

            <div
                class="p-6 rounded-3xl bg-slate-800/40 border border-slate-700/50 backdrop-blur-sm transition-all hover:border-red-500/30"
            >
                <div
                    class="text-slate-400 text-sm font-medium mb-1 uppercase tracking-tight"
                >
                    Process Failures
                </div>
                <div class="text-4xl font-black text-white">
                    {importStatus.failure}
                </div>
            </div>

            {#if error}
                <div
                    class="col-span-full p-4 rounded-2xl bg-red-500/10 border border-red-500/20 text-red-400 text-sm flex items-start gap-3"
                >
                    <i class="fa-solid fa-circle-info shrink-0 mt-0.5"></i>
                    <p>{error}</p>
                </div>
            {/if}
        </div>

        <!-- Empty State / Placeholder for Grid -->
        <div
            class="mt-12 flex-1 rounded-3xl border-2 border-dashed border-slate-800 flex flex-col items-center justify-center p-12 text-center"
        >
            <div
                class="w-20 h-20 rounded-full bg-slate-800 flex items-center justify-center mb-6 text-slate-600"
            >
                <i class="fa-solid fa-images text-4xl"></i>
            </div>
            <h3 class="text-xl font-bold text-slate-300 mb-2">
                No photos imported yet
            </h3>
            <p class="text-slate-500 max-w-xs">
                Start by importing a folder to see your collection organized by
                the core engine.
            </p>
        </div>
    </section>
</main>

<style>
    :global(body) {
        overflow: hidden;
    }
</style>
