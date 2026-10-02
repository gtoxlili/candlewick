import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { subscribe } from "@/lib/api";

/** Windows 11's caption glyphs, in Segoe Fluent Icons (Segoe MDL2 Assets on Windows 10). */
const GLYPH = {
  minimize: "",
  maximize: "",
  restore: "",
  close: "",
} as const;

/**
 * Minimize, maximize or restore, and close, drawn like Windows 11's own:
 * 46 px wide, the full title bar tall, a red close on hover. Out of the tab
 * order, as the system's are (Alt+Space and Alt+F4 reach them instead).
 */
export function CaptionButtons(props: { maximizable: boolean }) {
  const { t } = useTranslation();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!props.maximizable) return;
    const current = getCurrentWindow();
    const update = () => void current.isMaximized().then(setMaximized, () => {});
    update();
    return subscribe(current.onResized(update));
  }, [props.maximizable]);

  const current = getCurrentWindow();
  return (
    <div className="caption-buttons flex shrink-0 self-stretch">
      <CaptionButton
        label={t("windowControls.minimize")}
        glyph={GLYPH.minimize}
        onClick={() => void current.minimize()}
      />
      {props.maximizable && (
        <CaptionButton
          label={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
          glyph={maximized ? GLYPH.restore : GLYPH.maximize}
          onClick={() => void current.toggleMaximize()}
        />
      )}
      <CaptionButton
        label={t("windowControls.close")}
        glyph={GLYPH.close}
        close
        onClick={() => void current.close()}
      />
    </div>
  );
}

function CaptionButton(props: { label: string; glyph: string; close?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      tabIndex={-1}
      title={props.label}
      aria-label={props.label}
      data-close={props.close || undefined}
      onClick={props.onClick}
      className="caption-button grid w-[46px] place-items-center transition-colors duration-100"
    >
      <span aria-hidden>{props.glyph}</span>
    </button>
  );
}
