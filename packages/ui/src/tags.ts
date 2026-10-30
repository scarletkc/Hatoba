/** Tag dot colors. Common tags keep fixed colors; others hash onto the palette. */
const FIXED: Record<string, number> = {
  production: 1,
  prod: 1,
  staging: 2,
  database: 3,
  db: 3,
  edge: 4,
  api: 5,
  ci: 6,
  personal: 7,
};

export function tagColor(name: string): string {
  const key = name.toLowerCase();
  let slot = FIXED[key];
  if (!slot) {
    let h = 0;
    for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
    slot = (h % 8) + 1;
  }
  return `var(--tag-${slot})`;
}
