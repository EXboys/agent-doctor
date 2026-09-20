/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_AGENT_DOCTOR_EDITION?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
