// Minimal GitHub REST API client shared by CI and release scripts.

/** Creates a GitHub REST API client using `fetch`. */
export function githubClient(token, { userAgent, baseUrl } = {}) {
  const base = (baseUrl ?? process.env.GITHUB_API_URL ?? "https://api.github.com").replace(/\/+$/, "");

  async function request(method, path, body, { missingOk = false } = {}) {
    const headers = {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      ...(userAgent ? { "User-Agent": userAgent } : {}),
      ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
    };
    const res = await fetch(`${base}${path}`, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (missingOk && res.status === 404) return null;
    if (!res.ok) throw new Error(`${method} ${path}: HTTP ${res.status} ${await res.text()}`);
    return res.status === 204 ? null : res.json();
  }

  async function all(path) {
    const items = [];
    for (let page = 1; ; page++) {
      const batch = await request("GET", `${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
      items.push(...batch);
      if (batch.length < 100) return items;
    }
  }

  return { request, all };
}
