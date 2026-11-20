import { invoke } from "@tauri-apps/api/core";
import { createBrowserPreviewApi } from "./browserPreview";

export const isBrowserPreview =
  import.meta.env.DEV &&
  typeof window !== "undefined" &&
  !("__TAURI_INTERNALS__" in window);

export type TrustTier = "official" | "community_verified" | "unverified";
export type CompatibilityStatus = "verified" | "experimental" | "broken" | "untested";

export type PermissionScope =
  | "process.discover"
  | "process.read_memory"
  | "process.write_memory"
  | "process.allocate_memory"
  | "process.suspend_resume"
  | "filesystem.read_save_dir"
  | "filesystem.write_save_dir"
  | "runtime.native_module";

export interface DetectedGame {
  matched_game_id: string | null;
  title: string;
  store_source: "steam" | "gog" | "epic" | "xbox" | "custom_path";
  store_app_id: string | null;
  store_build_id: string | null;
  install_dir: string;
  executable_path: string | null;
  platform: string;
}

export interface LibraryGameView {
  library_key: string;
  game: DetectedGame;
  trainer_count: number;
}

export interface PermissionDeclaration {
  scope: PermissionScope;
  reason: string;
  required: boolean;
}

export type TrainerPermissionState =
  | { status: "approved"; granted_scopes: PermissionScope[] }
  | { status: "requires_confirmation"; missing_confirmations: PermissionDeclaration[] }
  | { status: "blocked"; reason: string };

export interface TrainerFeature {
  id: string;
  name: string;
  description: string;
  category: string;
  control_type: "toggle" | "action" | "integer" | "float";
  default_hotkey: string | null;
  mutates_memory: boolean;
  supports_rollback: boolean;
  risk_level: "low" | "medium" | "high";
}

export interface TrainerDetailView {
  trainer_id: string;
  name: string;
  version: string;
  description: string;
  authors: string[];
  source_repository: string;
  trust_tier: TrustTier;
  manifest_sha256: string;
  compatibility_status: CompatibilityStatus;
  compatibility_notes: string | null;
  revoked: boolean;
  permissions: PermissionDeclaration[];
  permission_state: TrainerPermissionState;
  features: TrainerFeature[];
}

export interface CustomGamePathRecord {
  path: string;
  label: string | null;
}

export interface SecurityPolicyConfig {
  allow_third_party_native_modules: boolean;
  allow_unverified_sources: boolean;
}

export interface DiagnosticEvent {
  level: string;
  category: string;
  message: string;
}

export interface GameProfile {
  game_id: string;
  trainer_id: string;
  hotkeys: Record<string, string>;
  saved_feature_states: Record<string, unknown>;
  presets?: Record<string, Record<string, unknown>>;
}

export interface ProcessIdentity {
  pid: number;
  process_name: string;
  executable_path: string;
  platform: string;
  architecture: string;
  executable_sha256: string | null;
  detected_version: string | null;
  process_creation_time_100ns: number | null;
}

export type FeatureValue =
  | { type: "toggle"; value: boolean }
  | { type: "action" }
  | { type: "integer"; value: number }
  | { type: "float"; value: number };

export interface FeatureCommand {
  feature_id: string;
  value: FeatureValue;
}

export interface RuntimeDiagnostics {
  session_id: string;
  runtime_kind: string;
  state: "initializing" | "attached" | "suspended" | "stopped" | "faulted";
  active_patches: number;
  rollback_failures: number;
  rollback_conflicts: number;
  active_freeze_features: number;
  target_pid: number | null;
  target_process_name: string | null;
  recent_events: string[];
}

export interface RegistrySource {
  repository_id: string;
  index_url: string;
  authoritative_trust_tier: TrustTier;
  enabled: boolean;
}

export interface RegistrySourceState {
  source: RegistrySource;
  etag: string | null;
  last_modified: string | null;
  last_synced_at: number | null;
  last_status: string;
  last_error: string | null;
}

export interface RegistrySyncReport {
  sources_checked: number;
  updated: number;
  not_modified: number;
  failed: number;
  revoked_trainer_ids: string[];
  errors: string[];
}

export type CommunityTrainerInstallState =
  | { state: "not_installed" }
  | { state: "installed_up_to_date"; installed_version: string }
  | { state: "update_available"; installed_version: string; latest_version: string }
  | { state: "revoked" };

export interface CommunityTrainerView {
  repository_id: string;
  trainer_id: string;
  game_id: string;
  latest_version: string;
  manifest_url: string;
  manifest_sha256: string | null;
  trust_tier: TrustTier;
  compatibility_status: CompatibilityStatus | null;
  last_verified_game_version: string | null;
  install_state: CommunityTrainerInstallState;
}

export interface TrainerUpdateInfo {
  repository_id: string;
  trainer_id: string;
  game_id: string;
  installed_version: string;
  available_version: string;
  trust_tier: TrustTier;
}

export interface DetectedLoader {
  kind:
    | "bep_in_ex_mono"
    | "bep_in_ex_il2cpp"
    | "melon_loader"
    | "nekod_unity_bridge"
    | "unity_explorer_mcp_bridge"
    | "ue4ss"
    | "re_framework"
    | "script_hook_v"
    | "script_hook_rdr2";
  entry_path: string;
  detected_version: string | null;
}

export interface GameEnvironmentProfile {
  install_dir: string;
  dir_exists: boolean;
  engine:
    | "unity_mono"
    | "unity_il2cpp"
    | "unreal_engine"
    | "re_engine"
    | "godot"
    | "native_unknown";
  installed_loaders: DetectedLoader[];
  anti_cheat_files_present: string[];
  recommended_adapters: string[];
}

export interface CompatibilityRecord {
  record_id: number;
  game_id: string;
  game_version: string;
  store_platform: string;
  runtime_adapter: string;
  runtime_version: string | null;
  mod_id: string;
  mod_version: string;
  status: CompatibilityStatus;
  evidence: string;
  notes: string | null;
  recorded_at: number;
}

const tauriApi = {
  rescanGames: () => invoke<LibraryGameView[]>("rescan_games"),
  listLibraryGames: () => invoke<LibraryGameView[]>("list_library_games"),
  listTrainersForGame: (gameId: string, detectedBuildId: string | null) =>
    invoke<TrainerDetailView[]>("list_trainers_for_game", {
      gameId,
      detectedBuildId,
    }),
  grantTrainerPermissions: (trainerId: string, confirmedScopes: PermissionScope[]) =>
    invoke<TrainerPermissionState>("grant_trainer_permissions", {
      trainerId,
      confirmedScopes,
    }),
  revokeTrainerPermissions: (trainerId: string) =>
    invoke<void>("revoke_trainer_permissions", { trainerId }),
  ingestLocalTrainer: (packageDir: string, trustTier: TrustTier) =>
    invoke<void>("ingest_local_trainer", { packageDir, trustTier }),
  registerGameManifest: (rawJson: string) =>
    invoke<string>("register_game_manifest", { rawJson }),
  addCustomGamePath: (path: string, label: string | null) =>
    invoke<CustomGamePathRecord[]>("add_custom_game_path", { path, label }),
  removeCustomGamePath: (path: string) =>
    invoke<CustomGamePathRecord[]>("remove_custom_game_path", { path }),
  listCustomGamePaths: () => invoke<CustomGamePathRecord[]>("list_custom_game_paths"),
  getSecurityPolicy: () => invoke<SecurityPolicyConfig>("get_security_policy"),
  updateSecurityPolicy: (policy: SecurityPolicyConfig) =>
    invoke<void>("update_security_policy", { policy }),
  loadProfile: (gameId: string, trainerId: string) =>
    invoke<GameProfile | null>("load_profile", { gameId, trainerId }),
  saveProfile: (profile: GameProfile) => invoke<void>("save_profile", { profile }),
  exportProfileJson: (gameId: string, trainerId: string) =>
    invoke<string>("export_profile_json", { gameId, trainerId }),
  importProfileJson: (rawJson: string) => invoke<GameProfile>("import_profile_json", { rawJson }),
  getDiagnosticLogs: () => invoke<DiagnosticEvent[]>("get_diagnostic_logs"),
  discoverRunningProcesses: (gameId: string) =>
    invoke<ProcessIdentity[]>("discover_running_processes", { gameId }),
  startTrainerSession: (
    gameId: string,
    trainerId: string,
    targetProcess: ProcessIdentity,
  ) =>
    invoke<string>("start_trainer_session", {
      gameId,
      trainerId,
      targetProcess,
    }),
  applyTrainerFeature: (sessionId: string, command: FeatureCommand) =>
    invoke<void>("apply_trainer_feature", { sessionId, command }),
  stopTrainerSession: (sessionId: string) =>
    invoke<RuntimeDiagnostics>("stop_trainer_session", { sessionId }),
  getSessionDiagnostics: (sessionId: string) =>
    invoke<RuntimeDiagnostics>("get_session_diagnostics", { sessionId }),
  listRegistrySources: () => invoke<RegistrySourceState[]>("list_registry_sources"),
  addThirdPartyRegistrySource: (repositoryId: string, indexUrl: string, enabled: boolean) =>
    invoke<void>("add_third_party_registry_source", {
      repositoryId,
      indexUrl,
      enabled,
    }),
  removeRegistrySource: (repositoryId: string) =>
    invoke<void>("remove_registry_source", { repositoryId }),
  syncRegistrySources: () => invoke<RegistrySyncReport>("sync_registry_sources"),
  listCommunityTrainersForGame: (gameId: string) =>
    invoke<CommunityTrainerView[]>("list_community_trainers_for_game", { gameId }),
  checkTrainerUpdates: () => invoke<TrainerUpdateInfo[]>("check_trainer_updates"),
  installOrUpdateTrainerFromRegistry: (repositoryId: string, trainerId: string) =>
    invoke<void>("install_or_update_trainer_from_registry", {
      repositoryId,
      trainerId,
    }),
  inspectGameEnvironment: (installDir: string) =>
    invoke<GameEnvironmentProfile>("inspect_game_environment", { installDir }),
  queryCompatibilityMatrix: (
    gameId: string,
    modId: string | null,
    gameVersion: string | null,
  ) =>
    invoke<CompatibilityRecord[]>("query_compatibility_matrix", {
      gameId,
      modId,
      gameVersion,
    }),
};

export const nekodApi = isBrowserPreview ? createBrowserPreviewApi() : tauriApi;
