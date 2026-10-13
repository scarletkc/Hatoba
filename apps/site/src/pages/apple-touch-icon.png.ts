import type { APIRoute } from "astro";
import { appIconPng } from "../lib/icons";

/** iOS rounds the corners itself and fills transparent ones with black, so the corners take the night sky's color. */
export const GET: APIRoute = async () => new Response(await appIconPng(180, "#0d1735"), { headers: { "content-type": "image/png" } });
