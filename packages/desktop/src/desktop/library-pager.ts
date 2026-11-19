/**
 * Paged access to the library list (desktop drawer).
 *
 * Rust groups, filters and sorts; this keeps only the pages around what is on
 * screen. After a library change the old pages stay as placeholders until
 * their replacements arrive, so the grid does not blink during an import.
 */

import { queryLibrary, type LibraryItem, type LocationFilter, type MediaFilter } from "../shared/query";
import type { SortBy } from "../shared/types";

const PAGE_SIZE = 200;
const MAX_PAGES = 30;

export interface LibraryCounts {
    /** Entries matching the filter. */
    total: number;
    /** Entries after RAW+JPEG grouping, before the filter. */
    groupedTotal: number;
    /** Rows in the database. */
    filesTotal: number;
}

export class LibraryPager {
    counts: LibraryCounts = { total: 0, groupedTotal: 0, filesTotal: 0 };
    /** At least one page has arrived for the current parameters. */
    loaded = false;

    private pages = new Map<number, LibraryItem[]>();
    private stale = new Map<number, LibraryItem[]>();
    private loading = new Map<number, Promise<void>>();
    private epoch = 0;
    private sortBy: SortBy = "date";
    private descending = true;
    private filter: MediaFilter = "all";
    private location: LocationFilter = "all";

    /** `onChange` fires whenever counts or pages change. */
    constructor(private onChange: () => void) {}

    /** Switch sort/filter. Nothing from the old order is reusable. */
    setParams(sortBy: SortBy, descending: boolean, filter: MediaFilter, location: LocationFilter = "all") {
        if (
            sortBy === this.sortBy && descending === this.descending && filter === this.filter
            && location === this.location && this.loaded
        ) {
            return;
        }
        this.location = location;
        this.sortBy = sortBy;
        this.descending = descending;
        this.filter = filter;
        this.reset();
    }

    /** The library changed: refetch, showing the old pages meanwhile. */
    invalidate() {
        this.epoch++;
        for (const [page, items] of this.pages) this.stale.set(page, items);
        this.pages.clear();
        this.loading.clear();
        void this.load(0);
    }

    /** Drop everything (after a delete, stale pages would show the deleted item). */
    reset() {
        this.epoch++;
        this.pages.clear();
        this.stale.clear();
        this.loading.clear();
        this.loaded = false;
        void this.load(0);
    }

    /** The item at `index` if it is cached; otherwise starts loading its page. */
    peek(index: number): LibraryItem | undefined {
        const page = Math.floor(index / PAGE_SIZE);
        const offset = index - page * PAGE_SIZE;
        const fresh = this.pages.get(page);
        if (fresh) {
            // Touch for LRU.
            this.pages.delete(page);
            this.pages.set(page, fresh);
            return fresh[offset];
        }
        void this.load(page);
        return this.stale.get(page)?.[offset];
    }

    /** The item at `index`, waiting for its page if needed. */
    async item(index: number): Promise<LibraryItem | undefined> {
        const page = Math.floor(index / PAGE_SIZE);
        if (!this.pages.has(page)) await this.load(page);
        return this.pages.get(page)?.[index - page * PAGE_SIZE];
    }

    private load(page: number): Promise<void> {
        const pending = this.loading.get(page);
        if (pending) return pending;
        const epoch = this.epoch;
        const request = queryLibrary(this.sortBy, this.descending, this.filter, this.location, page * PAGE_SIZE, PAGE_SIZE)
            .then((res) => {
                if (epoch !== this.epoch) return;
                this.counts = {
                    total: res.total,
                    groupedTotal: res.grouped_total,
                    filesTotal: res.files_total,
                };
                this.loaded = true;
                this.pages.set(page, res.items);
                this.stale.delete(page);
                while (this.pages.size > MAX_PAGES) {
                    const oldest = this.pages.keys().next().value as number;
                    this.pages.delete(oldest);
                }
                // Stale pages past the new end will never be replaced.
                const lastPage = Math.floor(Math.max(0, res.total - 1) / PAGE_SIZE);
                for (const key of this.stale.keys()) {
                    if (key > lastPage) this.stale.delete(key);
                }
                this.onChange();
            })
            .catch((e) => {
                console.error("[library] query_library failed", e);
            })
            .finally(() => {
                if (epoch === this.epoch) this.loading.delete(page);
            });
        this.loading.set(page, request);
        return request;
    }
}
