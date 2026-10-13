import type { APIRoute } from "astro";
import { appIcon } from "../lib/icons";

export const GET: APIRoute = () => new Response(new Uint8Array(appIcon()), { headers: { "content-type": "image/svg+xml" } });
