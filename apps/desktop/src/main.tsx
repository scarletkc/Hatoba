import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { boot } from "./app/boot";
import "./styles/base.css";

void boot().finally(async () => {
  // The component gallery (`/?gallery`) exists only in development builds.
  const Root =
    import.meta.env.DEV && new URLSearchParams(location.search).has("gallery") ? (await import("./app/Gallery")).Gallery : App;
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <Root />
    </StrictMode>,
  );
});
