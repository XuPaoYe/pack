/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_SUPERAI_PUBLIC_BUILD?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
