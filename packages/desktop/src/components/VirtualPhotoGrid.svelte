<script lang="ts">
    import { createVirtualizer } from "@tanstack/svelte-virtual";
    import ThumbnailImage from "./ThumbnailImage.svelte";
    import type { PhotoInfo } from "../types";

    let { photos, uniqueTs, thumbSize = 200, onPhotoClick } =
        $props<{
            photos: PhotoInfo[];
            uniqueTs: number;
            thumbSize?: number;
            onPhotoClick: (photo: PhotoInfo) => void;
        }>();

    let scrollElement: HTMLDivElement | undefined = $state();
    let containerWidth = $state(0);

    const GAP = 8;

    let columns = $derived(
        Math.max(1, Math.floor((containerWidth + GAP) / (thumbSize + GAP))),
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
                        class="grid gap-2"
                        style="grid-template-columns: repeat({columns}, 1fr);"
                    >
                        {#each rowPhotos as photo (photo.id.id)}
                            <div
                                class="aspect-square rounded bg-neutral-800 overflow-hidden cursor-pointer hover:ring-2 hover:ring-neutral-600 transition-all relative"
                                onclick={() => onPhotoClick(photo)}
                            >
                                <ThumbnailImage
                                    path={photo.thumb_path || photo.path}
                                    refreshKey={uniqueTs}
                                    alt="Photo thumbnail"
                                    className="w-full h-full"
                                    lazy={true}
                                />
                                {#if photo.hasRaw}
                                    <div class="absolute top-1 right-1 px-1.5 py-0.5 bg-amber-600 text-white text-[10px] font-bold rounded shadow">
                                        RAW
                                    </div>
                                {/if}
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
        background: #404040;
        border-radius: 10px;
    }
    .custom-scrollbar::-webkit-scrollbar-thumb:hover {
        background: #525252;
    }
</style>
