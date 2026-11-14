/**
 * Base map style shared by the desktop and mobile maps.
 *
 * Tiles: OpenStreetMap standard raster tiles (no API key). They only come in
 * one light style, so the dark theme is made from the same tiles with raster
 * paint properties: lightness is inverted (brightness-min above
 * brightness-max) and the hue turned half a circle first, so colours keep
 * their hue: light land becomes dark, dark labels become light, water stays
 * blue. Dimming the light tiles instead only looks like a grey veil over the
 * map.
 *
 * Usage policy: https://operations.osmfoundation.org/policies/tiles/
 * Standard tiles are 256px (@1x) only, which also satisfies the mobile
 * GL-memory rule against @2x tiles.
 */

import type { StyleSpecification } from "maplibre-gl";

export const GLYPHS_URL = "https://demotiles.maplibre.org/font/{fontstack}/{range}.pbf";

const OSM_TILES = ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"];
const OSM_ATTRIBUTION =
    '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors';

// App background colours (app.css: --bg-primary).
const DARK_BACKGROUND = "#0f172a";
const LIGHT_BACKGROUND = "#f8fafc";

const DARK_PAINT = {
    "raster-hue-rotate": 180,
    "raster-brightness-min": 0.92,
    "raster-brightness-max": 0.1,
    "raster-saturation": -0.55,
    "raster-contrast": 0.1,
    "raster-opacity": 0.85,
};

// A little calmer than the stock tiles, so photo markers stand out.
const LIGHT_PAINT = {
    "raster-saturation": -0.25,
};

export function baseMapStyle(
    isDark: boolean,
    custom?: { tiles: string[]; tileSize: number } | null,
): StyleSpecification {
    return {
        version: 8,
        glyphs: GLYPHS_URL,
        sources: {
            basemap: {
                type: "raster",
                tiles: custom?.tiles ?? OSM_TILES,
                tileSize: custom?.tileSize ?? 256,
                maxzoom: 19,
                attribution: OSM_ATTRIBUTION,
            },
        },
        layers: [
            // What shows before tiles arrive, and what the dark map is blended
            // onto so that it leans towards the app's background colour.
            {
                id: "basemap-background",
                type: "background",
                paint: { "background-color": isDark ? DARK_BACKGROUND : LIGHT_BACKGROUND },
            },
            {
                id: "basemap-layer",
                type: "raster",
                source: "basemap",
                paint: isDark ? DARK_PAINT : LIGHT_PAINT,
            },
        ],
    };
}
