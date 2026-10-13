import type { APIRoute } from "astro";
import { appIcon } from "../lib/icons";

export const GET: APIRoute = () => new Response(new Uint8Array(appIcon("icons/icon.ico")), { headers: { "content-type": "image/x-icon" } });
