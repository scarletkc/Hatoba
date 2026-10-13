import type { APIRoute } from "astro";
import { appIconPng } from "../lib/icons";

/** Search engines want a favicon of at least 48 pixels. */
export const GET: APIRoute = async () => new Response(await appIconPng(96), { headers: { "content-type": "image/png" } });
