import { useMemo, type ReactNode } from "react";
import { UiStringsProvider, type UiStrings } from "@hatoba/ui";
import { useT } from "@/i18n";

/** The words @hatoba/ui shows itself, in the app's language. */
export function AppUiStrings({ children }: { children: ReactNode }) {
  const t = useT();
  const strings = useMemo<UiStrings>(
    () => ({
      show: t("field.show"),
      hide: t("field.hide"),
      removeTag: (name) => t("field.removeTag", { name }),
      cancel: t("btn.cancel"),
    }),
    [t],
  );
  return <UiStringsProvider strings={strings}>{children}</UiStringsProvider>;
}
