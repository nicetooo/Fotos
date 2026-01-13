import { mount } from "svelte";
import { platform } from "@tauri-apps/plugin-os";
import { detectAndCreatePlatformService } from "./lib/platform";
import "./App.css";

async function bootstrap() {
    // Detect platform once at startup
    const currentPlatform = await platform();
    const isMobile = currentPlatform === "ios" || currentPlatform === "android";

    console.log("[Bootstrap] Platform:", currentPlatform, "isMobile:", isMobile);

    // Create platform service before mounting App
    const platformService = await detectAndCreatePlatformService();
    console.log("[Bootstrap] Platform service created:", platformService.platform);

    // Dynamic import based on platform
    const App = isMobile
        ? (await import("./mobile/App.svelte")).default
        : (await import("./desktop/App.svelte")).default;

    mount(App, {
        target: document.getElementById("app")!,
        props: { platformService },
    });
}

bootstrap().catch(console.error);
