import type { D1Migration } from "cloudflare:test";
import type { Env as Bindings } from "../src/index";

declare global {
  namespace Cloudflare {
    interface Env extends Bindings {
      /** Parsed migrations/*.sql, injected by vitest.config.ts. */
      TEST_MIGRATIONS: D1Migration[];
    }
    interface GlobalProps {
      mainModule: typeof import("../src/index");
    }
  }
}
