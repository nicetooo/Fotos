<script lang="ts">
    import { createVirtualizer } from "@tanstack/svelte-virtual";
    import ThumbnailImage from "./ThumbnailImage.svelte";
    import type { PhotoInfo } from "../types";

    let { photos, uniqueTs, onPhotoClick, onShowInFinder, formatDate } =
        $props<{
            photos: PhotoInfo[];
            uniqueTs: number;
            onPhotoClick: (photo: PhotoInfo) => void;
            onShowInFinder: (path: string, e: MouseEvent) => void;
            formatDate: (dateStr?: string) => string;
        }>();

    let scrollElement: HTMLDivElement | undefined = $state();
    let containerWidth = $state(0);

    // Calculate grid columns based on container width
    const ITEM_MIN_WIDTH = 200; // Minimum width for each item
    const GAP = 16; // Gap between items in pixels

    let columns = $derived(
        Math.max(
            1,
            Math.floor((containerWidth + GAP) / (ITEM_MIN_WIDTH + GAP)),
        ),
    );
    let itemWidth = $derived((containerWidth - GAP * (columns - 1)) / columns);

    // Calculate rows needed
    let rows = $derived(Math.ceil(photos.length / columns));

    // Create virtualizer for rows
    let rowVirtualizer = $derived(
        scrollElement
            ? createVirtualizer({
                  count: rows,
                  getScrollElement: () => scrollElement!,
                  estimateSize: () => itemWidth + GAP, // Row height = item width (square) + gap
                  overscan: 2, // Render 2 extra rows above and below viewport
              })
            : null,
    );

    // Observe container width changes
    let resizeObserver: ResizeObserver | undefined;

    $effect(() => {
        if (scrollElement) {
            resizeObserver = new ResizeObserver((entries) => {
                for (const entry of entries) {
                    containerWidth = entry.contentRect.width;
                }
            });
            resizeObserver.observe(scrollElement);

            return () => {
                resizeObserver?.disconnect();
            };
        }
    });

    // Get photos for a specific row
    function getPhotosForRow(rowIndex: number): PhotoInfo[] {
        const start = rowIndex * columns;
        const end = Math.min(start + columns, photos.length);
        return photos.slice(start, end);
    }
</script>

<div
    bind:this={scrollElement}
    class="flex-1 overflow-y-auto pr-2 custom-scrollbar"
>
    {#if rowVirtualizer}
        {@const virtualItems = $rowVirtualizer!.getVirtualItems()}
        {@const totalSize = $rowVirtualizer!.getTotalSize()}
        <div style="height: {totalSize}px; position: relative; width: 100%;">
            {#each virtualItems as virtualRow (virtualRow.key)}
                {@const rowPhotos = getPhotosForRow(virtualRow.index)}
                <div
                    style="position: absolute; top: 0; left: 0; width: 100%; transform: translateY({virtualRow.start}px);"
                    data-index={virtualRow.index}
                >
                    <div
                        class="grid gap-4"
                        style="grid-template-columns: repeat({columns}, 1fr);"
                    >
                        {#each rowPhotos as photo (photo.id.id)}
                            <div
                                class="aspect-square rounded-2xl bg-slate-800 overflow-hidden group relative border border-slate-700/50 hover:border-indigo-500/50 transition-all shadow-lg cursor-pointer"
                                onclick={() => onPhotoClick(photo)}
                            >
                                <ThumbnailImage
                                    path={photo.thumb_path || photo.path}
                                    refreshKey={uniqueTs}
                                    alt="Photo thumbnail"
                                    className="w-full h-full object-cover group-hover:scale-110 transition-transform duration-500"
                                    lazy={true}
                                />
                                <div
                                    class="absolute inset-0 bg-gradient-to-t from-black/80 via-black/20 to-transparent opacity-0 group-hover:opacity-100 transition-opacity p-3 flex flex-col justify-between"
                                >
                                    <div class="flex justify-end">
                                        <button
                                            onclick={(e) =>
                                                onShowInFinder(photo.path, e)}
                                            class="p-1.5 rounded-full bg-black/40 hover:bg-black/60 text-white/80 hover:text-white transition-colors backdrop-blur-sm"
                                            title="Show in Finder"
                                        >
                                            <i
                                                class="fa-solid fa-folder-open text-[10px]"
                                            ></i>
                                        </button>
                                    </div>

                                    <div class="flex flex-col gap-0.5">
                                        <p
                                            class="text-[10px] text-white font-bold truncate"
                                        >
                                            {photo.path.split("/").pop()}
                                        </p>

                                        <div
                                            class="flex items-center gap-2 text-[9px] text-slate-300 font-mono opacity-90"
                                        >
                                            <span>
                                                {photo.metadata.width}x{photo
                                                    .metadata.height}
                                            </span>
                                            <span class="uppercase">
                                                {photo.path.split(".").pop()}
                                            </span>
                                        </div>

                                        {#if photo.metadata.date_taken}
                                            <div
                                                class="flex items-center gap-1 text-[9px] text-indigo-300"
                                            >
                                                <i
                                                    class="fa-regular fa-calendar text-[8px]"
                                                ></i>
                                                <span
                                                    >{formatDate(
                                                        photo.metadata
                                                            .date_taken,
                                                    )}</span
                                                >
                                            </div>
                                        {/if}

                                        {#if photo.metadata.iso || photo.metadata.f_number}
                                            <div
                                                class="flex items-center gap-2 text-[8px] text-slate-400 mt-0.5"
                                            >
                                                {#if photo.metadata.iso}
                                                    <span
                                                        class="bg-slate-700/50 px-1 rounded"
                                                        >ISO {photo.metadata
                                                            .iso}</span
                                                    >
                                                {/if}
                                                {#if photo.metadata.f_number}
                                                    <span
                                                        >ƒ/{photo.metadata
                                                            .f_number}</span
                                                    >
                                                {/if}
                                            </div>
                                        {/if}
                                    </div>
                                </div>
                            </div>
                        {/each}
                    </div>
                </div>
            {/each}
        </div>
    {/if}
</div>

<style>
    .custom-scrollbar::-webkit-scrollbar {
        width: 6px;
    }
    .custom-scrollbar::-webkit-scrollbar-track {
        background: transparent;
    }
    .custom-scrollbar::-webkit-scrollbar-thumb {
        background: #1e293b;
        border-radius: 10px;
    }
    .custom-scrollbar::-webkit-scrollbar-thumb:hover {
        background: #334155;
    }
</style>
