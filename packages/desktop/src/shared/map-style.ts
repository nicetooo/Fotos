/**
 * Base map style shared by the desktop and mobile maps.
 *
 * Tiles: OpenStreetMap standard raster tiles (no API key). They only come in
 * one light style, so the dark theme dims and desaturates them with raster
 * paint properties instead of switching tile sets.
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
            {
                id: "basemap-layer",
                type: "raster",
                source: "basemap",
                paint: isDark
                    ? {
                          "raster-brightness-max": 0.42,
                          "raster-saturation": -0.55,
                          "raster-contrast": 0.1,
                      }
                    : {},
            },
        ],
    };
}
