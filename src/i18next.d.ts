import type { resources } from "@/lib/i18n";

// Keys `t` accepts: the English catalog's (src/lib/i18n.ts); a key that
// isn't there doesn't compile.
declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: (typeof resources)["en"];
    interpolationPrefix: "%{";
    interpolationSuffix: "}";
  }
}
