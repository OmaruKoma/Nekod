import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { PermissionScope, TrustTier } from "../api/nekod";

export type Locale = "zh-CN" | "en-US" | "ja-JP";

export const LOCALE_STORAGE_KEY = "nekod.locale.v1";

export const LOCALE_OPTIONS: {
  id: Locale;
  shortLabel: string;
  nativeLabel: string;
}[] = [
  { id: "zh-CN", shortLabel: "中文", nativeLabel: "简体中文" },
  { id: "en-US", shortLabel: "EN", nativeLabel: "English" },
  { id: "ja-JP", shortLabel: "日本語", nativeLabel: "日本語" },
];

export function detectInitialLocale(): Locale {
  try {
    const saved = localStorage.getItem(LOCALE_STORAGE_KEY);
    if (saved === "zh-CN" || saved === "en-US" || saved === "ja-JP") {
      return saved;
    }
  } catch {
    // Ignore storage access errors
  }
  const nav =
    typeof navigator !== "undefined" ? navigator.language.toLowerCase() : "";
  if (nav.startsWith("ja")) return "ja-JP";
  if (nav.startsWith("en")) return "en-US";
  return "zh-CN";
}

export interface TranslationMessages {
  titlebar: {
    primaryNavAria: string;
    workbench: string;
    store: string;
    importTab: string;
    settings: string;
    logs: string;
    activeSessionTitle: string;
    activeModsSuffix: (count: number) => string;
    toggleHud: string;
    soundOn: string;
    soundOff: string;
    soundToggleAria: string;
    languageTitle: string;
    minimize: string;
    minimizeAria: string;
    restore: string;
    maximize: string;
    restoreAria: string;
    maximizeAria: string;
    closeToTray: string;
    closeWindow: string;
    closeWindowAria: string;
  };
  sidebar: {
    heading: string;
    addLocalGameTitle: string;
    addLocalGameAria: string;
    rescanTitle: string;
    rescanAria: string;
    searchPlaceholder: string;
    filterAria: string;
    filterAll: string;
    filterFavorites: string;
    filterRunning: string;
    gamesHeader: (count: number) => string;
    injectedMany: (count: number) => string;
    injectedOne: string;
    emptyFilter: string;
    unfavorite: string;
    favorite: string;
  };
  workbench: {
    selectLeftPrompt: string;
    openStoreBtn: string;
    unfavorite: string;
    favorite: string;
    confirmRemove: string;
    cancel: string;
    cannotRemoveWhileInjected: string;
    removeCustomTitle: string;
    removeCustomBtn: string;
    bridgeDeployedBadge: (count: number) => string;
    antiCheatPrefix: string;
    copyPathTitle: string;
    playAndInjectTitle: string;
    launching: string;
    playAndInject: string;
    oneClickInjectTitle: string;
    injecting: string;
    oneClickInject: string;
    stackInjectTitle: string;
    stackInject: string;
    uninjectCurrentTitle: string;
    uninjecting: string;
    uninjectCurrent: string;
    uninjectSingle: string;
    uninjectAllTitle: string;
    uninjectAll: (count: number) => string;
    updateBridgeInjectedTitle: string;
    updateBridgeReadyTitle: string;
    updatingBridge: string;
    updateBridge: string;
    undeployBridgeTitle: string;
    uninstallingBridge: string;
    undeployBridge: string;
    deployBridgeTitle: string;
    deployingBridge: string;
    deployBridge: string;
    autoInjectTitle: string;
    autoInject: string;
    hudOverlayTitle: string;
    hudOverlay: string;
    soundTitle: string;
    sound: string;
    hotkeysCount: (count: number) => string;
    noProcessHint: string;
    scanProcessesTitle: string;
    waitingProcessHint: string;
    patchesCount: (n: number) => string;
    freezesCount: (n: number) => string;
    refreshDiagTitle: string;
    otherGameRunning: string;
    detachOtherSession: string;
    missingUnrealLoader: string;
    missingUnityLoader: string;
    installingLoader: string;
    installLoaderBtn: (name: string) => string;
    localNmodFound: string;
    featuresCount: (count: number) => string;
    importing: string;
    updateToVersion: (ver: string) => string;
    importNmod: string;
    modFeaturesTitle: string;
    initializing: string;
    quickSetupUnity: string;
    quickSetupUe4ss: string;
    noModUnityHint: string;
    noModUnrealHint: string;
    noModGenericHint: string;
    exportNmodTitle: string;
    exportingNmod: string;
    exportNmod: string;
    deletingMod: string;
    confirmDeleteMod: string;
    cannotDeleteWhileInjected: string;
    deleteModTitle: string;
    deleteMod: string;
    permBannerTitle: string;
    permBlockedPrefix: string;
    approveAndInject: string;
    approveOnly: string;
    modConsoleTitle: string;
    hotkeyConflict: (keys: string) => string;
    crossModConflict: (keys: string) => string;
    resetDefaultsTitle: string;
    resetBtn: string;
    saveProfileTitle: string;
    saveProfileBtn: string;
    filterAll: (n: number) => string;
    filterToggle: (n: number) => string;
    filterValue: (n: number) => string;
    filterAction: (n: number) => string;
    featureSearchPlaceholder: string;
    stackHint: string;
    injectFirstHint: string;
    closeJsonEditorAria: string;
    importJsonBtn: string;
    recordHotkeyTitle: string;
    recordingKey: string;
    bindHotkey: string;
    executing: string;
    stateOn: string;
    stateOff: string;
    triggerNow: string;
    applyValue: string;
    moreOptionsTitle: string;
    detailsToggleTitle: string;
    idleHintShort: string;
    expandDescHint: string;
    collapseDescHint: string;
    menuSectionBridge: string;
    menuSectionToggles: string;
    menuSectionGame: string;
    bridgeDrawerTitle: string;
    bridgeStatusDeployed: string;
    bridgeStatusIdle: string;
    ue4ssLoaderReady: string;
    unityLoaderReady: string;
    loaderMissingFallback: string;
    installUe4ssBtn: string;
    installLoaderSimple: string;
    uninstallingLoader: string;
    uninstallLoader: string;
    autoDeployOneClick: string;
    undeployAndRestore: string;
    noBridgeNeeded: (engineLabel: string) => string;
    ledgerFiles: (files: string) => string;
    allBuiltinBridges: (count: number) => string;
    manualSourcePlaceholder: string;
    manualTargetPlaceholder: string;
    manualDeploy: string;
    permissionsDrawerTitle: (count: number) => string;
    revokeAuth: string;
    liveSessionEventsTitle: (sid: string) => string;
    compatMatrixTitle: (count: number) => string;
    noCompatRecords: string;
    communityModsTitle: (count: number) => string;
    syncing: string;
    syncCommunity: string;
    noCommunityMods: string;
    installing: string;
    install: string;
    installed: string;
    updating: string;
    revoked: string;
  };
  store: {
    title: string;
    badge: string;
    subtitlePrefix: string;
    subtitleMiddle: string;
    subtitleSuffix: string;
    syncingAll: string;
    syncAll: string;
    updatesAvailable: (count: number) => string;
    updatesReconfirmHint: string;
    updating: string;
    updateTo: (ver: string) => string;
    filterAll: string;
    filterNotInstalled: string;
    filterInstalled: string;
    filterUpdates: string;
    searchPlaceholder: string;
    emptyTitle: string;
    emptyHintPrefix: string;
    emptyHintSuffix: string;
    verifiedGameVer: (ver: string) => string;
    installing: string;
    installNmod: string;
    installed: string;
    sourcesDrawer: (count: number) => string;
    removeRepo: string;
    addThirdPartyTitle: string;
    addThirdPartyDescPrefix: string;
    addThirdPartyDescMiddle: string;
    addThirdPartyDescSuffix: string;
    repoIdPlaceholder: string;
    repoUrlPlaceholder: string;
    addRepoBtn: string;
  };
  importCenter: {
    backToWorkbench: string;
    title: string;
    dropHintPill: string;
    sec0Title: string;
    sec0DescPrefix: string;
    sec0DescSuffix: string;
    pathPlaceholder: string;
    chooseFolder: string;
    chooseExe: string;
    labelPlaceholder: string;
    importing: string;
    addToLibrary: string;
    sec1TitlePrefix: string;
    sec1TitleMid: string;
    sec1TitleSuffix: string;
    sec1DescPrefix: string;
    sec1DescSuffix: string;
    nmodPlaceholder: string;
    chooseNmod: string;
    verifyAndImport: string;
    sec2TitlePrefix: string;
    sec2TitleSuffix: string;
    sec2Badge: string;
    sec2Desc1: string;
    sec2Desc2: string;
    sec2Desc3: string;
    customTargetOption: string;
    trustUnverifiedOpt: string;
    trustCommunityOpt: string;
    trustOfficialOpt: string;
    customGameIdPlaceholder: string;
    customGameNamePlaceholder: string;
    ctPathPlaceholder: string;
    chooseCt: string;
    defaultModulePlaceholder: string;
    convertingCt: string;
    convertAndImportCt: string;
    ctUnverifiedHintPrefix: string;
    ctUnverifiedHintSuffix: string;
    openInWorkbench: string;
    convertedFeatures: (count: number) => string;
    skippedEntries: (count: number) => string;
    unnamedEntry: string;
    sec3Title: string;
    gameManifestPlaceholder: string;
    verifyAndRegisterGame: string;
  };
  settings: {
    backToWorkbench: string;
    title: string;
    languageSection: string;
    languageLabel: string;
    languageDesc: string;
    desktopBehavior: string;
    minimizeToTrayTitle: string;
    minimizeToTrayDesc: string;
    autoAttachTitle: string;
    autoAttachDesc: string;
    hudOverlayTitle: string;
    hudOverlayDesc: string;
    soundTitle: string;
    soundDesc: string;
    securitySection: string;
    allowUnverifiedTitle: string;
    allowUnverifiedDesc: string;
    allowNativeTitle: string;
    allowNativeDesc: string;
    customFoldersTitle: string;
    pathPlaceholder: string;
    chooseFolder: string;
    chooseExe: string;
    labelPlaceholder: string;
    addFolderBtn: string;
    removeBtn: string;
    bridgesMatrixTitle: (count: number) => string;
  };
  logs: {
    backToWorkbench: string;
    title: string;
    exportBundle: string;
    refresh: string;
    closeBundleAria: string;
    emptyLogs: string;
  };
  modals: {
    addGameTitle: string;
    closeAria: string;
    addGameDescPrefix: string;
    pathPlaceholder: string;
    chooseFolder: string;
    chooseExe: string;
    labelPlaceholder: string;
    importing: string;
    addToLibrary: string;
    dropDefaultPreview: string;
    dropTitle: string;
    dropDesc1: string;
    dropDesc2: string;
    dropDesc3: string;
  };
  app: {
    browserPreviewBanner: string;
    closeErrorAria: string;
    closeStatusAria: string;
    copiedPathToast: string;
    installPathStatus: (path: string) => string;
    autoAttachOn: string;
    autoAttachOff: string;
    hudOverlayOn: string;
    hudOverlayOff: string;
    trayOn: string;
    trayOff: string;
    rescanSummary: (count: number) => string;
    ctDropToast: string;
    dropImportOkToast: string;
    dropImportStatus: (path: string) => string;
    antiCheatRejectInject: string;
    noProcessError: string;
    autoInjectSuccess: (name: string, proc: string, pid: number) => string;
    injectSuccess: (name: string, proc: string, pid: number) => string;
    waitingBridgeReady: (proc: string, pid: number) => string;
    otherGameSessionActive: string;
    trainerAlreadyRunning: (name: string) => string;
    trainerBlocked: (reason: string) => string;
    requiresConfirmationStatus: (name: string) => string;
    antiCheatRejectLaunch: string;
    bridgeAutoUpdatedToast: string;
    launchedNeedUnityStarter: (target: string) => string;
    launchedNeedUe4ssStarter: (target: string) => string;
    launchedNeedMod: (target: string) => string;
    launchedWaitingInject: (target: string) => string;
    permissionsApprovedStatus: (name: string, ver: string) => string;
    uninjectSingleStatus: (
      name: string,
      conflicts: number,
      failures: number,
    ) => string;
    uninjectMultiStatus: (
      count: number,
      conflicts: number,
      failures: number,
    ) => string;
    gameExitedStatus: (title: string) => string;
    bridgeAutoDeployed: (engine: string, files: string) => string;
    manualBridgeInputError: string;
    bridgeManualDeployed: (target: string) => string;
    bridgeUndeployed: (
      removed: number,
      restored: number,
      conflicts: number,
    ) => string;
    loaderInitProgress: string;
    loaderInstalledNeedBridge: (label: string) => string;
    loaderInstalledNeedRestart: (label: string) => string;
    loaderInstalledToast: (shortLabel: string) => string;
    loaderUndeployed: (
      shortLabel: string,
      removed: number,
      restored: number,
      conflicts: number,
    ) => string;
    unityStarterDoneStatus: (tid: string) => string;
    unityStarterDoneToast: string;
    ue4ssStarterDoneStatus: (tid: string) => string;
    ue4ssStarterDoneToast: string;
    nmodInstalledStatus: (name: string, count: number) => string;
    nmodInstalledToast: (name: string) => string;
    invalidNumberError: (name: string) => string;
    featureToggledOn: (name: string, suffix: string) => string;
    featureToggledOff: (name: string, suffix: string) => string;
    featureTriggered: (name: string, suffix: string) => string;
    permissionsRevoked: (name: string) => string;
    customGameImported: (title: string) => string;
    customGameAdded: string;
    customGameRemoved: string;
    uninstallStopFirstError: string;
    trainerUninstalled: (name: string) => string;
    nmodExportedStatus: (path: string) => string;
    nmodExportedToast: string;
    localTrainerIngested: string;
    gameManifestRegistered: (gid: string) => string;
    ctImportedToast: (count: number) => string;
    ctSkippedToast: (count: number) => string;
    syncComplete: (
      checked: number,
      updated: number,
      unchanged: number,
      failed: number,
      revoked: number,
    ) => string;
    repoAdded: string;
    repoRemoved: (id: string) => string;
    communityInstalledStatus: (tid: string) => string;
    communityInstalledToast: (tid: string) => string;
    hotkeyBoundToast: (hk: string) => string;
    hotkeyClearedToast: string;
    profileSavedStatus: (name: string) => string;
    profileSavedToast: string;
    profileResetStatus: (name: string) => string;
    presetAppliedStatus: (preset: string) => string;
    profileExportedStatus: (name: string) => string;
    profileImportedStatus: (tid: string) => string;
    bundleGeneratedStatus: string;
    unknownEngine: string;
    pickFolderTitle: string;
    pickExeTitle: string;
    pickNmodTitle: string;
    pickNmodFolderTitle: string;
    pickCtTitle: string;
  };
}

const MESSAGES: Record<Locale, TranslationMessages> = {
  "zh-CN": {
    titlebar: {
      primaryNavAria: "主导航",
      workbench: "工作台",
      store: "模组商店",
      importTab: "导入与转换",
      settings: "设置",
      logs: "诊断日志",
      activeSessionTitle: "跳转到当前注入中的游戏工作台",
      activeModsSuffix: (n) => `${n} Mod 运行中`,
      toggleHud: "切换游戏内 RGB 悬浮窗",
      soundOn: "提示音：开启",
      soundOff: "提示音：静音",
      soundToggleAria: "切换提示音",
      languageTitle: "切换界面语言",
      minimize: "最小化",
      minimizeAria: "最小化窗口",
      restore: "还原窗口",
      maximize: "最大化",
      restoreAria: "还原窗口",
      maximizeAria: "最大化窗口",
      closeToTray: "隐藏到系统托盘（保持全局热键生效）",
      closeWindow: "关闭 Nekod",
      closeWindowAria: "关闭窗口",
    },
    sidebar: {
      heading: "游戏库",
      addLocalGameTitle: "添加本地游戏（选择目录或 .exe）",
      addLocalGameAria: "添加本地游戏",
      rescanTitle: "重新扫描游戏目录",
      rescanAria: "重新扫描游戏目录",
      searchPlaceholder: "快速切换游戏...",
      filterAria: "游戏列表筛选",
      filterAll: "全部",
      filterFavorites: "收藏",
      filterRunning: "运行",
      gamesHeader: (n) => `游戏 (${n})`,
      injectedMany: (n) => `已注入 (${n})`,
      injectedOne: "已注入",
      emptyFilter: "当前筛选下无匹配游戏。",
      unfavorite: "取消收藏",
      favorite: "收藏置顶",
    },
    workbench: {
      selectLeftPrompt: "请从左侧游戏库选择一款游戏以进入工作台，或前往模组商店安装新包。",
      openStoreBtn: "前往模组商店",
      unfavorite: "取消收藏",
      favorite: "收藏置顶",
      confirmRemove: "确认移除",
      cancel: "取消",
      cannotRemoveWhileInjected: "请先解除注入后再移除本地游戏",
      removeCustomTitle: "从游戏库移除此自定义游戏（不删除本地文件）",
      removeCustomBtn: "移除本地游戏",
      bridgeDeployedBadge: (n) => `桥接已部署 (${n})`,
      antiCheatPrefix: "反作弊",
      copyPathTitle: "点击复制安装目录路径",
      playAndInjectTitle: "启动游戏并在检测到进程后自动注入已授权 Mod",
      launching: "启动中...",
      playAndInject: "启动游戏并注入",
      oneClickInjectTitle: "连接到当前已经在运行的游戏进程并激活 Mod",
      injecting: "注入中...",
      oneClickInject: "一键注入 Mod",
      stackInjectTitle: "在当前运行中的同一游戏上叠加注入此 Mod",
      stackInject: "叠加注入当前 Mod",
      uninjectCurrentTitle: "仅解除并回滚当前选中的 Mod",
      uninjecting: "回滚中...",
      uninjectCurrent: "解除当前 Mod",
      uninjectSingle: "解除注入",
      uninjectAllTitle: "按逆序全部解除并回滚当前游戏已注入的所有 Mod",
      uninjectAll: (n) => `解除全部 (${n})`,
      updateBridgeInjectedTitle: "请先解除注入并关闭游戏后再更新桥接",
      updateBridgeReadyTitle: "检测到新版伴生桥接，点击一键覆盖升级",
      updatingBridge: "更新中...",
      updateBridge: "更新伴生桥接",
      undeployBridgeTitle: "按 DeploymentLedger 卸载游戏目录中的伴生桥接文件",
      uninstallingBridge: "卸载中...",
      undeployBridge: "解除桥接",
      deployBridgeTitle: "自动识别引擎并一键部署伴生桥接",
      deployingBridge: "部署中...",
      deployBridge: "部署伴生桥接",
      autoInjectTitle: "检测到已授权游戏进程启动时自动注入",
      autoInject: "自动注入",
      hudOverlayTitle: "在游戏画面右上角置顶显示 RGB 流光英文已开启功能列表",
      hudOverlay: "RGB 悬浮窗",
      soundTitle: "开关与注入提示音效",
      sound: "提示音",
      hotkeysCount: (n) => `${n} hotkeys`,
      noProcessHint: "未检测到进程（可点击「启动游戏并注入」或手动启动游戏）",
      scanProcessesTitle: "扫描运行中的游戏进程",
      waitingProcessHint: "等待进程出现后自动注入...",
      patchesCount: (n) => `patches ${n}`,
      freezesCount: (n) => `freezes ${n}`,
      refreshDiagTitle: "刷新会话状态",
      otherGameRunning: "另一款游戏正在注入中",
      detachOtherSession: "解除另一会话",
      missingUnrealLoader: "缺少 Unreal Engine 引导加载器",
      missingUnityLoader: "缺少 Unity 引导加载器",
      installingLoader: "下载并校验安装中...",
      installLoaderBtn: (name) => `安装 ${name}`,
      localNmodFound: "发现匹配的本地 .nmod 模组包",
      featuresCount: (n) => `${n} 项功能`,
      importing: "导入中...",
      updateToVersion: (v) => `更新至 v${v}`,
      importNmod: "导入 .nmod",
      modFeaturesTitle: "模组功能",
      initializing: "初始化中...",
      quickSetupUnity: "一键初始化通用 Unity 控制模组",
      quickSetupUe4ss: "一键初始化通用 UE4SS 控制模组",
      noModUnityHint:
        "检测到当前为 Unity 引擎游戏，尚未绑定专用模组。可拖入或导入 .nmod 模组包，或在右上角「一键初始化通用 Unity 控制模组」（Pause F8 / 游戏变速 / 资源清理）。",
      noModUnrealHint:
        "检测到当前为 Unreal Engine 游戏，尚未绑定专用模组。先安装 RE-UE4SS 与伴生桥接，再点击「一键初始化通用 UE4SS 控制模组」或前往「模组商店」安装专属包。",
      noModGenericHint:
        "当前游戏尚未安装本地模组。可展开下方「社区可用模组」、前往顶部「模组商店」安装，或直接将 .nmod 文件拖入窗口。",
      exportNmodTitle: "将该模组打包导出为单文件 .nmod 压缩包",
      exportingNmod: "导出中...",
      exportNmod: "导出 .nmod",
      deletingMod: "删除中...",
      confirmDeleteMod: "确认删除",
      cannotDeleteWhileInjected: "请先解除注入后再删除模组",
      deleteModTitle: "从本地模组库卸载并删除此模组",
      deleteMod: "删除",
      permBannerTitle: "注入前权限确认 (Default-Deny)",
      permBlockedPrefix: "已阻止",
      approveAndInject: "确认权限并立即注入",
      approveOnly: "仅授权暂不注入",
      modConsoleTitle: "模组控制台",
      hotkeyConflict: (k) => `快捷键冲突: ${k}`,
      crossModConflict: (k) => `跨模组热键重叠: ${k}`,
      resetDefaultsTitle: "恢复默认开关与热键",
      resetBtn: "重置",
      saveProfileTitle: "保存当前热键与预设配置",
      saveProfileBtn: "保存配置",
      filterAll: (n) => `全部 (${n})`,
      filterToggle: (n) => `开关 (${n})`,
      filterValue: (n) => `数值 (${n})`,
      filterAction: (n) => `动作 (${n})`,
      featureSearchPlaceholder: "筛选功能名称或说明...",
      stackHint:
        "当前游戏已注入其他 Mod：点击上方「叠加注入当前 Mod」即可同时启用本 Mod 功能。",
      injectFirstHint:
        "点击上方「启动游戏并注入」或「一键注入 Mod」后即可切换开关并激活全局热键（点击右侧快捷键胶囊可自定义按键）。",
      closeJsonEditorAria: "关闭 JSON 编辑器",
      importJsonBtn: "导入 JSON 配置",
      recordHotkeyTitle: "点击后按下任意组合键绑定（Esc 取消 · Backspace 清除）",
      recordingKey: "按下按键...",
      bindHotkey: "绑定热键",
      executing: "执行中...",
      stateOn: "开启",
      stateOff: "关闭",
      triggerNow: "立即触发",
      applyValue: "应用",
      moreOptionsTitle: "更多选项与快捷设置",
      detailsToggleTitle: "点击展开或收起游戏路径与环境详情",
      idleHintShort: "未检测到运行进程 · 可点击「启动游戏并注入」",
      expandDescHint: "点击展开完整简介",
      collapseDescHint: "点击收起简介",
      menuSectionBridge: "伴生桥接",
      menuSectionToggles: "快捷开关",
      menuSectionGame: "本地游戏",
      bridgeDrawerTitle: "内置伴生桥接资源桶 (DeploymentLedger)",
      bridgeStatusDeployed: "deployed",
      bridgeStatusIdle: "idle",
      ue4ssLoaderReady: "RE-UE4SS 引导器已就绪，启动游戏时将自动加载 NekodBridge。",
      unityLoaderReady:
        "引导加载器已就绪，重启或运行游戏时将自动加载 NekodUnityBridge。",
      loaderMissingFallback:
        "缺少引导加载器，可通过 DeploymentLedger 一键安装与无痕卸载。",
      installUe4ssBtn: "一键安装 RE-UE4SS",
      installLoaderSimple: "安装加载器",
      uninstallingLoader: "卸载中...",
      uninstallLoader: "卸载加载器",
      autoDeployOneClick: "一键自动部署",
      undeployAndRestore: "卸载并还原",
      noBridgeNeeded: (engineLabel) =>
        `当前游戏引擎（${engineLabel}）无需伴生桥接，或可在下方手动指定自定义插件路径。`,
      ledgerFiles: (files) => `账本记录文件: ${files}`,
      allBuiltinBridges: (n) => `全部内置伴生桥接清单 (${n})`,
      manualSourcePlaceholder: "手动源路径（可选）",
      manualTargetPlaceholder: "游戏内目标相对路径",
      manualDeploy: "手动部署",
      permissionsDrawerTitle: (n) => `权限明细与撤销 (${n})`,
      revokeAuth: "撤销该模组授权",
      liveSessionEventsTitle: (sid) => `实时注入会话事件 (${sid})`,
      compatMatrixTitle: (n) => `兼容矩阵 (${n})`,
      noCompatRecords: "暂无已记录的版本兼容验证结果。",
      communityModsTitle: (n) => `社区可用模组 (${n})`,
      syncing: "同步中...",
      syncCommunity: "同步社区源",
      noCommunityMods:
        "当前游戏在已同步社区源中暂无模组，可前往顶部「模组商店」添加或同步。",
      installing: "安装中...",
      install: "安装",
      installed: "已安装",
      updating: "更新中...",
      revoked: "已吊销",
    },
    store: {
      title: "在线 .nmod 模组商店",
      badge: "Source-First · SHA-256 Verified",
      subtitlePrefix: "默认订阅官方",
      subtitleMiddle: "模组桶，也可添加任意第三方",
      subtitleSuffix: "源；下载后自动校验哈希并关联游戏识别规则。",
      syncingAll: "同步仓库索引...",
      syncAll: "同步全部仓库",
      updatesAvailable: (n) => `发现可用更新 (${n})`,
      updatesReconfirmHint: "更新后需重新确认声明权限",
      updating: "更新中...",
      updateTo: (v) => `更新至 v${v}`,
      filterAll: "全部模组",
      filterNotInstalled: "可安装",
      filterInstalled: "已安装",
      filterUpdates: "待更新",
      searchPlaceholder: "搜索游戏 slug、模组 ID 或源...",
      emptyTitle: "暂未同步到在线模组索引",
      emptyHintPrefix: "首次使用请点击右上角「同步全部仓库」拉取",
      emptyHintSuffix: "最新清单。",
      verifiedGameVer: (v) => `验证版本: ${v}`,
      installing: "安装中...",
      installNmod: "安装 .nmod",
      installed: "已安装",
      sourcesDrawer: (n) => `已订阅模组仓库源 (${n})`,
      removeRepo: "移除",
      addThirdPartyTitle: "添加第三方社区仓库",
      addThirdPartyDescPrefix: "输入 HTTPS 协议的",
      addThirdPartyDescMiddle: "地址，第三方源默认标记为",
      addThirdPartyDescSuffix: "。",
      repoIdPlaceholder: "repository-id",
      repoUrlPlaceholder: "https://raw.githubusercontent.com/.../registry.json",
      addRepoBtn: "添加仓库",
    },
    importCenter: {
      backToWorkbench: "返回工作台",
      title: "本地导入与 Cheat Engine 静态转换",
      dropHintPill: "支持拖入 .nmod / .zip / .ct 到任意页面",
      sec0Title: "0. 添加本地游戏（非 Steam / 绿色版 / 独立游戏）",
      sec0DescPrefix: "支持直接选择游戏根目录或主程序",
      sec0DescSuffix: "；添加后自动识别游戏引擎并支持一键启动与伴生桥接部署。",
      pathPlaceholder: "D:\\Games\\MyGame 或 D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "选择目录",
      chooseExe: "选择 .exe",
      labelPlaceholder: "可选显示名称（留空则自动识别游戏名）",
      importing: "正在导入...",
      addToLibrary: "添加到游戏库",
      sec1TitlePrefix: "1. 导入本地",
      sec1TitleMid: "模组包（单文件",
      sec1TitleSuffix: "或解包目录）",
      sec1DescPrefix: "导入前强制校验 JSON Schema 与构件 SHA-256；也可直接将",
      sec1DescSuffix: "文件拖拽到 Nekod 窗口中松开。",
      nmodPlaceholder: "C:\\Mods\\my-game.nmod 或 C:\\Mods\\my-nmod-dir",
      chooseNmod: "选择 .nmod",
      verifyAndImport: "校验并导入",
      sec2TitlePrefix: "2. 导入 Cheat Engine 表格 (",
      sec2TitleSuffix: "-> 声明式模组)",
      sec2Badge: "fail-closed static converter",
      sec2Desc1: "仅静态转换模块相对偏移、指针链与纯",
      sec2Desc2: "补丁，并生成带 SHA-256 校验的标准声明式模组包；任何",
      sec2Desc3: "或远程脚本将被安全拒绝。",
      customTargetOption: "手动指定目标游戏 ID / 名称...",
      trustUnverifiedOpt: "Unverified (需开启设置允许)",
      trustCommunityOpt: "Community Verified (已自行审计)",
      trustOfficialOpt: "Official",
      customGameIdPlaceholder: "目标游戏 canonical slug (可选，例如 sample-rpg)",
      customGameNamePlaceholder: "目标游戏显示名称 (例如 Sample RPG)",
      ctPathPlaceholder: "C:\\Users\\...\\Downloads\\Game.CT",
      chooseCt: "选择 .CT",
      defaultModulePlaceholder: "默认模块名 (可选，例如 Game.exe)",
      convertingCt: "静态解析中...",
      convertAndImportCt: "静态转换并导入",
      ctUnverifiedHintPrefix:
        "提示：Default-Deny 安全策略默认阻止 Unverified 模组授权；若需在本地调试未认证表格，请前往「设置」开启允许未认证来源，或自行审计后选择",
      ctUnverifiedHintSuffix: "。",
      openInWorkbench: "前往工作台查看",
      convertedFeatures: (n) => `已转换声明式功能 (${n})`,
      skippedEntries: (n) => `已安全拦截/跳过条目 (${n})`,
      unnamedEntry: "(未命名条目)",
      sec3Title: "3. 注册 Game Manifest JSON",
      gameManifestPlaceholder: "粘贴通过校验的 game-manifest.json...",
      verifyAndRegisterGame: "校验并注册游戏清单",
    },
    settings: {
      backToWorkbench: "返回工作台",
      title: "桌面端偏好与安全策略",
      languageSection: "界面语言 / Language",
      languageLabel: "显示语言",
      languageDesc: "即时切换桌面端全部菜单、提示与错误说明（支持中文 / English / 日本語）。",
      desktopBehavior: "桌面端体验与托盘常驻",
      minimizeToTrayTitle: "关闭主窗口时最小化到系统托盘（推荐）",
      minimizeToTrayDesc:
        "保持全局快捷键与游戏内 HUD 浮窗在后台正常运行；可右键托盘图标彻底退出。",
      autoAttachTitle: "自动附着已授权模组",
      autoAttachDesc:
        "检测到已选游戏主程序启动时自动连接并挂载（仅限已明确确认权限的模组）。",
      hudOverlayTitle: "开启游戏内 RGB 英文悬浮窗 (HUD ArrayList)",
      hudOverlayDesc:
        "置顶显示当前激活模组列表（全鼠标穿透；游戏需为窗口化或无边框模式）。",
      soundTitle: "开启注入与开关提示音效",
      soundDesc: "在热键切换功能或成功注入时播放轻量提示音。",
      securitySection: "Default-Deny 安全控制",
      allowUnverifiedTitle: "允许未认证 (Unverified) 来源的声明式模组",
      allowUnverifiedDesc:
        "开启后可手动确认第三方未审核来源的模组权限（仍强制展示 SHA-256 与声明范围）。",
      allowNativeTitle: "允许第三方原生运行时模块 (高风险，默认禁用)",
      allowNativeDesc: "仅在你完全信任本地原生模块代码时才建议打开。",
      customFoldersTitle: "自定义游戏目录 / 本地游戏",
      pathPlaceholder: "D:\\Games\\MyGame 或 D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "选择目录",
      chooseExe: "选择 .exe",
      labelPlaceholder: "可选显示名称（留空则自动识别游戏名）",
      addFolderBtn: "添加目录",
      removeBtn: "移除",
      bridgesMatrixTitle: (n) => `内置伴生桥接版本矩阵 (${n})`,
    },
    logs: {
      backToWorkbench: "返回工作台",
      title: "诊断与审计日志",
      exportBundle: "导出诊断包",
      refresh: "刷新",
      closeBundleAria: "收起诊断包",
      emptyLogs: "暂无诊断事件记录。",
    },
    modals: {
      addGameTitle: "添加本地游戏",
      closeAria: "关闭添加本地游戏窗口",
      addGameDescPrefix:
        "Steam / GOG / Epic 游戏会自动扫描识别。对于免安装绿色版或自定义目录游戏，可选择游戏根目录或直接选择主程序",
      pathPlaceholder: "D:\\Games\\MyGame 或 D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "选择目录",
      chooseExe: "选择 .exe",
      labelPlaceholder: "可选显示名称（留空则自动识别游戏名）",
      importing: "正在导入...",
      addToLibrary: "添加到游戏库",
      dropDefaultPreview: ".nmod / .zip / .ct / 文件夹",
      dropTitle: "松开即可校验并导入模组",
      dropDesc1: "支持单文件",
      dropDesc2: "归档包、模组目录或 Cheat Engine",
      dropDesc3: "表格",
    },
    app: {
      browserPreviewBanner:
        "浏览器预览模式：当前展示模拟 IPC 数据；运行 Tauri 桌面端可连接真实 Windows 进程、Steam 封面与全局热键。",
      closeErrorAria: "关闭错误提示",
      closeStatusAria: "关闭状态提示",
      copiedPathToast: "已复制安装目录路径",
      installPathStatus: (p) => `安装路径: ${p}`,
      autoAttachOn: "已开启自动附着：检测到已授权游戏启动将自动注入",
      autoAttachOff: "已关闭自动附着模式",
      hudOverlayOn: "已开启 RGB 英文 HUD 悬浮窗（需游戏为窗口化/无边框窗口化）",
      hudOverlayOff: "已关闭 RGB 英文 HUD 悬浮窗",
      trayOn: "关闭窗口将隐藏到系统托盘（保持热键运行）",
      trayOff: "关闭窗口将直接退出 Nekod",
      rescanSummary: (n) => `已重新扫描游戏库：发现 ${n} 款已安装游戏。`,
      ctDropToast: "已载入 .CT 表格，请在「导入与转换」确认目标游戏",
      dropImportOkToast: "已完成 SHA-256 校验并导入本地模组",
      dropImportStatus: (p) => `已通过拖拽校验并导入模组：${p}`,
      antiCheatRejectInject:
        "检测到反作弊保护标记，根据安全策略拒绝注入该游戏进程（Nekod 仅支持单机/无反作弊环境）。",
      noProcessError:
        "未检测到运行中的游戏进程。请先启动游戏（或直接点「启动游戏并注入」）。",
      autoInjectSuccess: (n, p, pid) =>
        `已自动注入「${n}」 -> ${p} (PID ${pid})`,
      injectSuccess: (n, p, pid) => `已注入「${n}」 -> ${p} (PID ${pid})`,
      waitingBridgeReady: (p, pid) =>
        `检测到游戏进程 (${p} · PID ${pid})，正在等待伴生桥接就绪...`,
      otherGameSessionActive:
        "当前另一款游戏已有活跃注入会话，请先点击「解除注入」后再切换其他游戏。",
      trainerAlreadyRunning: (n) => `「${n}」已在当前游戏运行中。`,
      trainerBlocked: (r) => `当前模组被安全策略阻止：${r}`,
      requiresConfirmationStatus: (n) =>
        `「${n}」需要确认声明的运行权限，请在下方点击「确认权限并立即注入」。`,
      antiCheatRejectLaunch:
        "检测到反作弊保护标记，无法对该游戏启用「启动游戏并注入」。",
      bridgeAutoUpdatedToast: "检测到新版伴生桥接，已自动更新后再启动游戏",
      launchedNeedUnityStarter: (t) =>
        `已启动游戏 (${t})。请先点击「一键初始化通用 Unity 控制模组」或导入模组。`,
      launchedNeedUe4ssStarter: (t) =>
        `已启动游戏 (${t})。请先点击「一键初始化通用 UE4SS 控制模组」或导入模组。`,
      launchedNeedMod: (t) => `已启动游戏 (${t})。请先安装本地 .nmod 模组后再注入。`,
      launchedWaitingInject: (t) =>
        `已启动游戏 (${t})，正在等待进程出现后自动注入...`,
      permissionsApprovedStatus: (n, v) => `已授权 ${n} v${v} 的声明权限。`,
      uninjectSingleStatus: (n, c, f) =>
        `已解除「${n}」并按 LIFO 顺序完成回滚（冲突: ${c}，失败: ${f}）`,
      uninjectMultiStatus: (cnt, c, f) =>
        `已全部解除 ${cnt} 个模组并按 LIFO 顺序完成回滚（冲突: ${c}，失败: ${f}）`,
      gameExitedStatus: (t) => `检测到「${t}」已退出，已自动断开注入会话。`,
      bridgeAutoDeployed: (e, files) =>
        `已自动识别 ${e} 并部署伴生桥接 (${files})`,
      manualBridgeInputError:
        "请填写本地源插件路径与目标相对路径，或直接点击「一键自动部署」。",
      bridgeManualDeployed: (target) =>
        `已通过 DeploymentLedger 部署伴生桥接文件到 ${target}。`,
      bridgeUndeployed: (rem, res, con) =>
        `已解除伴生桥接：移除 ${rem} 个文件，还原 ${res} 个原文件（冲突: ${con}）`,
      loaderInitProgress: "正在初始化异步下载与安装任务...",
      loaderInstalledNeedBridge: (l) =>
        `已安装 ${l}。接下来点击「部署伴生桥接」并启动/重启游戏即可生效。`,
      loaderInstalledNeedRestart: (l) =>
        `已安装 ${l}。若游戏正在运行，请重启一次游戏即可自动加载桥接插件。`,
      loaderInstalledToast: (s) => `${s} 加载器安装完成（重启游戏后生效）`,
      loaderUndeployed: (s, rem, res, con) =>
        `已卸载 ${s} 加载器：移除 ${rem} 个文件，还原 ${res} 个原文件（冲突: ${con}）`,
      unityStarterDoneStatus: (tid) =>
        `已初始化通用 Unity 控制模组 (${tid})！请先确认运行权限，然后点击「一键注入 Mod」。`,
      unityStarterDoneToast: "通用 Unity 控制模组初始化完成",
      ue4ssStarterDoneStatus: (tid) =>
        `已初始化通用 UE4SS 控制模组 (${tid})！请先确认运行权限，然后点击「一键注入 Mod」。`,
      ue4ssStarterDoneToast: "通用 UE4SS 控制模组初始化完成",
      nmodInstalledStatus: (n, c) =>
        `已导入 .nmod 模组「${n}」(${c} 项功能)！请先确认运行权限后注入。`,
      nmodInstalledToast: (n) => `已导入 .nmod：${n}`,
      invalidNumberError: (n) => `请为「${n}」输入有效数字。`,
      featureToggledOn: (n, s) => `${n} 已开启${s}`,
      featureToggledOff: (n, s) => `${n} 已关闭${s}`,
      featureTriggered: (n, s) => `已执行：${n}${s}`,
      permissionsRevoked: (n) => `已撤销「${n}」的运行权限。`,
      customGameImported: (t) => `已导入本地游戏「${t}」并加入游戏库。`,
      customGameAdded: "已添加自定义游戏路径并完成扫描。",
      customGameRemoved: "已从游戏库移除该自定义本地游戏（未删除游戏源文件）。",
      uninstallStopFirstError: "请先点击「解除注入」停止该模组后再删除。",
      trainerUninstalled: (n) => `已删除模组「${n}」（权限授权已撤销）。`,
      nmodExportedStatus: (p) => `已导出 .nmod 模组包至：${p}`,
      nmodExportedToast: "已完成 .nmod 单文件归档打包",
      localTrainerIngested: "已完成校验并导入本地模组包。",
      gameManifestRegistered: (gid) => `已注册游戏清单 '${gid}'。`,
      ctImportedToast: (n) => `已转换并导入 ${n} 项 .CT 修改功能`,
      ctSkippedToast: (n) => `未能导入 .CT：已安全拦截 ${n} 条非声明式条目`,
      syncComplete: (chk, upd, unch, fail, rev) =>
        `同步完成（检查 ${chk} 源）：${upd} 更新，${unch} 无变化，${fail} 失败，${rev} 已吊销。`,
      repoAdded: "已添加第三方社区源（默认 Unverified），请点击同步获取索引。",
      repoRemoved: (id) => `已移除社区源 '${id}'。`,
      communityInstalledStatus: (tid) =>
        `已完成 SHA-256 校验并安装 '${tid}'，注入前请确认其运行权限。`,
      communityInstalledToast: (tid) => `已安装在线模组：${tid}`,
      hotkeyBoundToast: (hk) => `快捷键已绑定为 ${hk}`,
      hotkeyClearedToast: "已清除该功能快捷键",
      profileSavedStatus: (n) => `已保存「${n}」的快捷键与预设配置。`,
      profileSavedToast: "已保存快捷键与预设配置",
      profileResetStatus: (n) => `已重置「${n}」的默认快捷键与开关状态。`,
      presetAppliedStatus: (p) => `已应用并保存预设「${p}」。`,
      profileExportedStatus: (n) => `已导出「${n}」的可移植 JSON 配置。`,
      profileImportedStatus: (tid) => `已导入 '${tid}' 的 JSON 配置。`,
      bundleGeneratedStatus: "已生成可审计诊断包 JSON。",
      unknownEngine: "未知",
      pickFolderTitle: "选择本地游戏根目录",
      pickExeTitle: "选择游戏主程序 (.exe)",
      pickNmodTitle: "选择 .nmod 模组归档包",
      pickNmodFolderTitle: "选择已解包的模组目录",
      pickCtTitle: "选择 Cheat Engine .CT 表格文件",
    },
  },

  "en-US": {
    titlebar: {
      primaryNavAria: "Primary navigation",
      workbench: "Workbench",
      store: "Mod Store",
      importTab: "Import & Convert",
      settings: "Settings",
      logs: "Logs",
      activeSessionTitle: "Jump to the currently injected game workbench",
      activeModsSuffix: (n) => `${n} mod${n > 1 ? "s" : ""} active`,
      toggleHud: "Toggle in-game RGB HUD overlay",
      soundOn: "Sound FX: On",
      soundOff: "Sound FX: Muted",
      soundToggleAria: "Toggle sound effects",
      languageTitle: "Switch display language",
      minimize: "Minimize",
      minimizeAria: "Minimize window",
      restore: "Restore window",
      maximize: "Maximize",
      restoreAria: "Restore window",
      maximizeAria: "Maximize window",
      closeToTray: "Minimize to system tray (keep global hotkeys active)",
      closeWindow: "Close Nekod",
      closeWindowAria: "Close window",
    },
    sidebar: {
      heading: "Library",
      addLocalGameTitle: "Add local game (folder or .exe)",
      addLocalGameAria: "Add local game",
      rescanTitle: "Rescan game library",
      rescanAria: "Rescan game library",
      searchPlaceholder: "Filter games...",
      filterAria: "Filter game list",
      filterAll: "All",
      filterFavorites: "Favs",
      filterRunning: "Live",
      gamesHeader: (n) => `Games (${n})`,
      injectedMany: (n) => `Injected (${n})`,
      injectedOne: "Injected",
      emptyFilter: "No matching games in this filter.",
      unfavorite: "Remove favorite",
      favorite: "Pin to top",
    },
    workbench: {
      selectLeftPrompt:
        "Select a game on the left to open the workbench, or visit Mod Store to install packages.",
      openStoreBtn: "Open Mod Store",
      unfavorite: "Remove favorite",
      favorite: "Pin to top",
      confirmRemove: "Confirm Remove",
      cancel: "Cancel",
      cannotRemoveWhileInjected: "Uninject active mods before removing this game",
      removeCustomTitle: "Remove custom entry from library (local files untouched)",
      removeCustomBtn: "Remove Entry",
      bridgeDeployedBadge: (n) => `Bridge Deployed (${n})`,
      antiCheatPrefix: "Anti-Cheat",
      copyPathTitle: "Click to copy install directory",
      playAndInjectTitle:
        "Launch the game and auto-inject authorized mods once the process starts",
      launching: "Launching...",
      playAndInject: "Play & Inject",
      oneClickInjectTitle: "Attach to the running game process and activate this mod",
      injecting: "Injecting...",
      oneClickInject: "Inject Mod",
      stackInjectTitle: "Stack this mod onto the currently running game session",
      stackInject: "Stack Mod",
      uninjectCurrentTitle: "Uninject and roll back only the selected mod",
      uninjecting: "Rolling back...",
      uninjectCurrent: "Uninject Mod",
      uninjectSingle: "Uninject",
      uninjectAllTitle: "Uninject and roll back all active mods in LIFO order",
      uninjectAll: (n) => `Uninject All (${n})`,
      updateBridgeInjectedTitle:
        "Uninject and close the game before updating the companion bridge",
      updateBridgeReadyTitle: "Newer companion bridge available; click to update",
      updatingBridge: "Updating...",
      updateBridge: "Update Bridge",
      undeployBridgeTitle:
        "Remove companion bridge files from game folder via DeploymentLedger",
      uninstallingBridge: "Removing...",
      undeployBridge: "Undeploy Bridge",
      deployBridgeTitle:
        "Detect game engine and deploy the matching companion bridge",
      deployingBridge: "Deploying...",
      deployBridge: "Deploy Bridge",
      autoInjectTitle: "Automatically attach when the game process starts",
      autoInject: "Auto-Attach",
      hudOverlayTitle:
        "Display click-through RGB HUD ArrayList at top-right of the game window",
      hudOverlay: "RGB HUD",
      soundTitle: "Toggle audio chimes on inject and feature switches",
      sound: "Audio",
      hotkeysCount: (n) => `${n} hotkeys`,
      noProcessHint:
        "No running process found (click Play & Inject or launch the game)",
      scanProcessesTitle: "Scan running game processes",
      waitingProcessHint: "Waiting for process to auto-inject...",
      patchesCount: (n) => `patches ${n}`,
      freezesCount: (n) => `freezes ${n}`,
      refreshDiagTitle: "Refresh session diagnostics",
      otherGameRunning: "Another game session is active",
      detachOtherSession: "Detach Other Session",
      missingUnrealLoader: "Missing Unreal Engine Bootstrap Loader",
      missingUnityLoader: "Missing Unity Bootstrap Loader",
      installingLoader: "Downloading & verifying...",
      installLoaderBtn: (name) => `Install ${name}`,
      localNmodFound: "Matched local .nmod packages",
      featuresCount: (n) => `${n} features`,
      importing: "Importing...",
      updateToVersion: (v) => `Update v${v}`,
      importNmod: "Import .nmod",
      modFeaturesTitle: "Mod Features",
      initializing: "Initializing...",
      quickSetupUnity: "Quick Setup Unity Starter Mod",
      quickSetupUe4ss: "Quick Setup UE4SS Starter Mod",
      noModUnityHint:
        "Unity engine detected without dedicated mods. Drop a .nmod package here or click Quick Setup Unity Starter Mod (Pause F8 / Speedhack / GC).",
      noModUnrealHint:
        "Unreal Engine detected without dedicated mods. Install RE-UE4SS + Bridge, then click Quick Setup UE4SS Starter Mod or browse Mod Store.",
      noModGenericHint:
        "No local mods installed for this game yet. Browse Community Mods below, open Mod Store, or drag & drop a .nmod archive anywhere.",
      exportNmodTitle: "Package and export this mod as a standalone .nmod archive",
      exportingNmod: "Exporting...",
      exportNmod: "Export .nmod",
      deletingMod: "Deleting...",
      confirmDeleteMod: "Confirm Delete",
      cannotDeleteWhileInjected: "Uninject this mod before uninstalling",
      deleteModTitle: "Uninstall and remove this mod from local storage",
      deleteMod: "Delete",
      permBannerTitle: "Permission Approval Required (Default-Deny)",
      permBlockedPrefix: "Blocked",
      approveAndInject: "Approve & Inject",
      approveOnly: "Approve Only",
      modConsoleTitle: "Mod Console",
      hotkeyConflict: (k) => `Hotkey conflict: ${k}`,
      crossModConflict: (k) => `Cross-mod overlap: ${k}`,
      resetDefaultsTitle: "Restore default toggles and hotkeys",
      resetBtn: "Reset",
      saveProfileTitle: "Save current hotkeys and preset states",
      saveProfileBtn: "Save Profile",
      filterAll: (n) => `All (${n})`,
      filterToggle: (n) => `Toggle (${n})`,
      filterValue: (n) => `Value (${n})`,
      filterAction: (n) => `Action (${n})`,
      featureSearchPlaceholder: "Filter features...",
      stackHint:
        "Another mod is active on this game: click Stack Mod above to enable both together.",
      injectFirstHint:
        "Click Play & Inject or Inject Mod above to toggle features and activate global hotkeys (click a hotkey capsule to rebind).",
      closeJsonEditorAria: "Close JSON editor",
      importJsonBtn: "Import JSON Profile",
      recordHotkeyTitle:
        "Click then press any key combo to bind (Esc cancel · Backspace clear)",
      recordingKey: "Press key...",
      bindHotkey: "Bind Key",
      executing: "Running...",
      stateOn: "ON",
      stateOff: "OFF",
      triggerNow: "Trigger",
      applyValue: "Apply",
      moreOptionsTitle: "More & Quick Settings",
      detailsToggleTitle: "Click to toggle install path & environment details",
      idleHintShort: "Idle · Click Play & Inject to launch",
      expandDescHint: "Click to expand description",
      collapseDescHint: "Click to collapse description",
      menuSectionBridge: "Companion Bridge",
      menuSectionToggles: "Quick Toggles",
      menuSectionGame: "Local Entry",
      bridgeDrawerTitle: "Companion Bridge Bucket (DeploymentLedger)",
      bridgeStatusDeployed: "deployed",
      bridgeStatusIdle: "idle",
      ue4ssLoaderReady:
        "RE-UE4SS loader is ready and will load NekodBridge when the game starts.",
      unityLoaderReady:
        "Loader is ready and will load NekodUnityBridge when the game runs.",
      loaderMissingFallback:
        "Loader missing; install and cleanly roll back via DeploymentLedger.",
      installUe4ssBtn: "Install RE-UE4SS",
      installLoaderSimple: "Install Loader",
      uninstallingLoader: "Removing...",
      uninstallLoader: "Uninstall Loader",
      autoDeployOneClick: "Auto Deploy",
      undeployAndRestore: "Undeploy & Restore",
      noBridgeNeeded: (engineLabel) =>
        `Detected engine (${engineLabel}) requires no built-in bridge, or specify custom plugin paths below.`,
      ledgerFiles: (files) => `Ledger files: ${files}`,
      allBuiltinBridges: (n) => `All Built-in Companion Bridges (${n})`,
      manualSourcePlaceholder: "Manual source path (optional)",
      manualTargetPlaceholder: "Target relative path inside game dir",
      manualDeploy: "Manual Deploy",
      permissionsDrawerTitle: (n) => `Permissions & Revocation (${n})`,
      revokeAuth: "Revoke Authorization",
      liveSessionEventsTitle: (sid) => `Live Session Events (${sid})`,
      compatMatrixTitle: (n) => `Compatibility Matrix (${n})`,
      noCompatRecords: "No compatibility verification records yet.",
      communityModsTitle: (n) => `Community Available Mods (${n})`,
      syncing: "Syncing...",
      syncCommunity: "Sync Sources",
      noCommunityMods:
        "No community mods indexed for this game yet. Sync or add repositories in Mod Store.",
      installing: "Installing...",
      install: "Install",
      installed: "Installed",
      updating: "Updating...",
      revoked: "Revoked",
    },
    store: {
      title: "Online .nmod Mod Store",
      badge: "Source-First · SHA-256 Verified",
      subtitlePrefix: "Subscribes to official",
      subtitleMiddle: "bucket by default; add any third-party",
      subtitleSuffix:
        "index with automatic SHA-256 integrity checks and game binding.",
      syncingAll: "Syncing registries...",
      syncAll: "Sync All Sources",
      updatesAvailable: (n) => `Updates Available (${n})`,
      updatesReconfirmHint: "Re-confirmation of permissions required after update",
      updating: "Updating...",
      updateTo: (v) => `Update v${v}`,
      filterAll: "All Mods",
      filterNotInstalled: "Available",
      filterInstalled: "Installed",
      filterUpdates: "Updates",
      searchPlaceholder: "Search game slug, mod ID, or repo...",
      emptyTitle: "No online mods indexed yet",
      emptyHintPrefix: "Click Sync All Sources in the top-right to fetch the latest",
      emptyHintSuffix: "index.",
      verifiedGameVer: (v) => `Verified build: ${v}`,
      installing: "Installing...",
      installNmod: "Install .nmod",
      installed: "Installed",
      sourcesDrawer: (n) => `Subscribed Registry Sources (${n})`,
      removeRepo: "Remove",
      addThirdPartyTitle: "Add Third-Party Community Registry",
      addThirdPartyDescPrefix: "Enter an HTTPS",
      addThirdPartyDescMiddle: "index URL. Third-party sources default to",
      addThirdPartyDescSuffix: ".",
      repoIdPlaceholder: "repository-id",
      repoUrlPlaceholder: "https://raw.githubusercontent.com/.../registry.json",
      addRepoBtn: "Add Source",
    },
    importCenter: {
      backToWorkbench: "Back to Workbench",
      title: "Local Import & Cheat Engine Static Converter",
      dropHintPill: "Drop .nmod / .zip / .ct anywhere in window",
      sec0Title: "0. Add Local Game (Non-Steam / Portable / DRM-Free)",
      sec0DescPrefix: "Choose a game root folder or main executable",
      sec0DescSuffix:
        "; Nekod detects the engine automatically and enables Play & Inject.",
      pathPlaceholder: "D:\\Games\\MyGame or D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "Select Folder",
      chooseExe: "Select .exe",
      labelPlaceholder: "Optional display title (auto-detected if empty)",
      importing: "Adding...",
      addToLibrary: "Add to Library",
      sec1TitlePrefix: "1. Import Local",
      sec1TitleMid: "Package (Single",
      sec1TitleSuffix: "Archive or Unpacked Dir)",
      sec1DescPrefix: "Validates JSON Schema and artifact SHA-256; you can also drop",
      sec1DescSuffix: "files directly onto the Nekod window.",
      nmodPlaceholder: "C:\\Mods\\my-game.nmod or C:\\Mods\\my-nmod-dir",
      chooseNmod: "Select .nmod",
      verifyAndImport: "Verify & Import",
      sec2TitlePrefix: "2. Import Cheat Engine Table (",
      sec2TitleSuffix: "-> Declarative Mod)",
      sec2Badge: "fail-closed static converter",
      sec2Desc1: "Statically converts relative module offsets, pointer chains, and",
      sec2Desc2: "patches into a SHA-256 verified package;",
      sec2Desc3: "or remote execution blocks are rejected fail-closed.",
      customTargetOption: "Specify custom target game ID / name...",
      trustUnverifiedOpt: "Unverified (requires policy opt-in)",
      trustCommunityOpt: "Community Verified (self-audited)",
      trustOfficialOpt: "Official",
      customGameIdPlaceholder: "Target canonical slug (optional, e.g. sample-rpg)",
      customGameNamePlaceholder: "Target display name (e.g. Sample RPG)",
      ctPathPlaceholder: "C:\\Users\\...\\Downloads\\Game.CT",
      chooseCt: "Select .CT",
      defaultModulePlaceholder: "Default module (optional, e.g. Game.exe)",
      convertingCt: "Converting...",
      convertAndImportCt: "Convert & Import",
      ctUnverifiedHintPrefix:
        "Note: Default-Deny blocks Unverified authorization by default; enable Unverified sources in Settings or audit first and select",
      ctUnverifiedHintSuffix: ".",
      openInWorkbench: "Open in Workbench",
      convertedFeatures: (n) => `Converted Features (${n})`,
      skippedEntries: (n) => `Rejected / Skipped Entries (${n})`,
      unnamedEntry: "(Unnamed entry)",
      sec3Title: "3. Register Game Manifest JSON",
      gameManifestPlaceholder: "Paste validated game-manifest.json...",
      verifyAndRegisterGame: "Verify & Register Game",
    },
    settings: {
      backToWorkbench: "Back to Workbench",
      title: "Preferences & Security Policy",
      languageSection: "Interface Language",
      languageLabel: "Display Language",
      languageDesc:
        "Switch all desktop labels, menus, and diagnostics immediately (中文 / English / 日本語).",
      desktopBehavior: "Desktop Experience & System Tray",
      minimizeToTrayTitle: "Minimize to system tray on close (Recommended)",
      minimizeToTrayDesc:
        "Keep global hotkeys and RGB HUD overlay active in background; right-click tray icon to quit.",
      autoAttachTitle: "Auto-Attach authorized mods",
      autoAttachDesc:
        "Automatically inject pre-authorized mods when the selected game process starts.",
      hudOverlayTitle: "Enable in-game RGB HUD ArrayList",
      hudOverlayDesc:
        "Click-through always-on-top active feature list (requires windowed or borderless mode).",
      soundTitle: "Enable audio cues on inject & toggle",
      soundDesc: "Play subtle synthesized tones when injecting or toggling hotkeys.",
      securitySection: "Default-Deny Security Controls",
      allowUnverifiedTitle: "Allow Unverified declarative mod sources",
      allowUnverifiedDesc:
        "Allows manual approval of unverified third-party packages (SHA-256 and scopes remain enforced).",
      allowNativeTitle: "Allow third-party native runtime modules (High Risk, Off)",
      allowNativeDesc:
        "Only enable if you fully trust local native binary modules.",
      customFoldersTitle: "Custom Game Folders / Local Executables",
      pathPlaceholder: "D:\\Games\\MyGame or D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "Select Folder",
      chooseExe: "Select .exe",
      labelPlaceholder: "Optional display title (auto-detected if empty)",
      addFolderBtn: "Add Path",
      removeBtn: "Remove",
      bridgesMatrixTitle: (n) => `Built-in Companion Bridges (${n})`,
    },
    logs: {
      backToWorkbench: "Back to Workbench",
      title: "Diagnostics & Audit Logs",
      exportBundle: "Export Bundle",
      refresh: "Refresh",
      closeBundleAria: "Close bundle viewer",
      emptyLogs: "No diagnostic events recorded yet.",
    },
    modals: {
      addGameTitle: "Add Local Game",
      closeAria: "Close add game dialog",
      addGameDescPrefix:
        "Steam / GOG / Epic games are discovered automatically. For portable or custom installs, pick the folder or main",
      pathPlaceholder: "D:\\Games\\MyGame or D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "Select Folder",
      chooseExe: "Select .exe",
      labelPlaceholder: "Optional display title (auto-detected if empty)",
      importing: "Importing...",
      addToLibrary: "Add to Library",
      dropDefaultPreview: ".nmod / .zip / .ct / Folder",
      dropTitle: "Drop to Verify & Import Mod",
      dropDesc1: "Supports",
      dropDesc2: "archives, unpacked mod directories, or Cheat Engine",
      dropDesc3: "tables",
    },
    app: {
      browserPreviewBanner:
        "Browser preview mode: showing simulated IPC data; run the Tauri desktop app for live Windows process injection and global hotkeys.",
      closeErrorAria: "Dismiss error",
      closeStatusAria: "Dismiss status",
      copiedPathToast: "Copied install path to clipboard",
      installPathStatus: (p) => `Install path: ${p}`,
      autoAttachOn: "Auto-Attach enabled: will inject when target process launches",
      autoAttachOff: "Auto-Attach disabled",
      hudOverlayOn: "RGB HUD Overlay enabled (requires windowed / borderless game)",
      hudOverlayOff: "RGB HUD Overlay disabled",
      trayOn: "Window close will hide to system tray (hotkeys stay active)",
      trayOff: "Window close will quit Nekod",
      rescanSummary: (n) => `Rescan complete: ${n} installed games found.`,
      ctDropToast: "Loaded .CT table; confirm target game in Import & Convert",
      dropImportOkToast: "Verified SHA-256 and imported local mod",
      dropImportStatus: (p) => `Verified and imported dropped package: ${p}`,
      antiCheatRejectInject:
        "Anti-cheat markers detected. Injection refused by security policy (Nekod supports offline singleplayer only).",
      noProcessError:
        "No running game process found. Launch the game first or click Play & Inject.",
      autoInjectSuccess: (n, p, pid) =>
        `Auto-injected "${n}" -> ${p} (PID ${pid})`,
      injectSuccess: (n, p, pid) => `Injected "${n}" -> ${p} (PID ${pid})`,
      waitingBridgeReady: (p, pid) =>
        `Game process detected (${p} · PID ${pid}), waiting for companion bridge...`,
      otherGameSessionActive:
        "Another game has an active session. Uninject it before switching.",
      trainerAlreadyRunning: (n) => `"${n}" is already active on this game.`,
      trainerBlocked: (r) => `Mod blocked by security policy: ${r}`,
      requiresConfirmationStatus: (n) =>
        `"${n}" requires permission approval. Click Approve & Inject below.`,
      antiCheatRejectLaunch:
        "Anti-cheat markers detected; Play & Inject is disabled for this game.",
      bridgeAutoUpdatedToast: "Updated companion bridge before launching game",
      launchedNeedUnityStarter: (t) =>
        `Launched (${t}). Click Quick Setup Unity Starter Mod or import a .nmod package.`,
      launchedNeedUe4ssStarter: (t) =>
        `Launched (${t}). Click Quick Setup UE4SS Starter Mod or import a .nmod package.`,
      launchedNeedMod: (t) =>
        `Launched (${t}). Install a local .nmod package before injecting.`,
      launchedWaitingInject: (t) =>
        `Launched (${t}), waiting for process to auto-inject...`,
      permissionsApprovedStatus: (n, v) =>
        `Approved permissions for ${n} v${v}.`,
      uninjectSingleStatus: (n, c, f) =>
        `Uninjected "${n}" with LIFO rollback (conflicts: ${c}, failures: ${f})`,
      uninjectMultiStatus: (cnt, c, f) =>
        `Uninjected ${cnt} mods with LIFO rollback (conflicts: ${c}, failures: ${f})`,
      gameExitedStatus: (t) =>
        `"${t}" exited; active injection session detached cleanly.`,
      bridgeAutoDeployed: (e, files) =>
        `Detected ${e} and deployed companion bridge (${files})`,
      manualBridgeInputError:
        "Specify both source path and target relative path, or use Auto Deploy.",
      bridgeManualDeployed: (target) =>
        `Deployed companion bridge to ${target} via DeploymentLedger.`,
      bridgeUndeployed: (rem, res, con) =>
        `Undeployed bridge: removed ${rem} files, restored ${res} originals (conflicts: ${con})`,
      loaderInitProgress: "Initializing loader download & verification...",
      loaderInstalledNeedBridge: (l) =>
        `Installed ${l}. Click Deploy Bridge then start/restart the game.`,
      loaderInstalledNeedRestart: (l) =>
        `Installed ${l}. Restart the game if already running to load the bridge.`,
      loaderInstalledToast: (s) => `${s} loader installed (restart game to apply)`,
      loaderUndeployed: (s, rem, res, con) =>
        `Removed ${s} loader: deleted ${rem} files, restored ${res} originals (conflicts: ${con})`,
      unityStarterDoneStatus: (tid) =>
        `Initialized Unity starter mod (${tid}). Approve permissions then click Inject Mod.`,
      unityStarterDoneToast: "Unity starter mod initialized",
      ue4ssStarterDoneStatus: (tid) =>
        `Initialized UE4SS starter mod (${tid}). Approve permissions then click Inject Mod.`,
      ue4ssStarterDoneToast: "UE4SS starter mod initialized",
      nmodInstalledStatus: (n, c) =>
        `Imported "${n}" (${c} features). Approve permissions to inject.`,
      nmodInstalledToast: (n) => `Imported .nmod: ${n}`,
      invalidNumberError: (n) => `Enter a valid number for "${n}".`,
      featureToggledOn: (n, s) => `${n} ON${s}`,
      featureToggledOff: (n, s) => `${n} OFF${s}`,
      featureTriggered: (n, s) => `Triggered: ${n}${s}`,
      permissionsRevoked: (n) => `Revoked permissions for "${n}".`,
      customGameImported: (t) => `Added local game "${t}" to library.`,
      customGameAdded: "Custom game path added and scanned.",
      customGameRemoved: "Removed custom entry from library (files preserved).",
      uninstallStopFirstError: "Uninject this mod before deleting it.",
      trainerUninstalled: (n) => `Deleted "${n}" (permissions revoked).`,
      nmodExportedStatus: (p) => `Exported .nmod archive to: ${p}`,
      nmodExportedToast: "Exported standalone .nmod archive",
      localTrainerIngested: "Verified and imported local mod package.",
      gameManifestRegistered: (gid) => `Registered game manifest '${gid}'.`,
      ctImportedToast: (n) => `Converted and imported ${n} .CT features`,
      ctSkippedToast: (n) =>
        `.CT import rejected: ${n} non-declarative entries blocked`,
      syncComplete: (chk, upd, unch, fail, rev) =>
        `Sync done (${chk} sources): ${upd} updated, ${unch} unchanged, ${fail} failed, ${rev} revoked.`,
      repoAdded: "Added third-party source (Unverified). Click Sync to fetch index.",
      repoRemoved: (id) => `Removed repository source '${id}'.`,
      communityInstalledStatus: (tid) =>
        `Verified SHA-256 and installed '${tid}'. Approve permissions before inject.`,
      communityInstalledToast: (tid) => `Installed online mod: ${tid}`,
      hotkeyBoundToast: (hk) => `Hotkey bound to ${hk}`,
      hotkeyClearedToast: "Cleared feature hotkey",
      profileSavedStatus: (n) => `Saved hotkeys & presets for "${n}".`,
      profileSavedToast: "Saved profile",
      profileResetStatus: (n) => `Reset defaults for "${n}".`,
      presetAppliedStatus: (p) => `Applied and saved preset "${p}".`,
      profileExportedStatus: (n) => `Exported portable profile JSON for "${n}".`,
      profileImportedStatus: (tid) => `Imported JSON profile for '${tid}'.`,
      bundleGeneratedStatus: "Generated auditable diagnostic bundle JSON.",
      unknownEngine: "Unknown",
      pickFolderTitle: "Select Game Root Directory",
      pickExeTitle: "Select Game Executable (.exe)",
      pickNmodTitle: "Select .nmod Package Archive",
      pickNmodFolderTitle: "Select Unpacked Mod Directory",
      pickCtTitle: "Select Cheat Engine .CT Table",
    },
  },

  "ja-JP": {
    titlebar: {
      primaryNavAria: "メインナビゲーション",
      workbench: "ワークベンチ",
      store: "Mod ストア",
      importTab: "インポート / 変換",
      settings: "設定",
      logs: "診断ログ",
      activeSessionTitle: "現在注入中のゲームワークベンチへ移動",
      activeModsSuffix: (n) => `${n} Mod 稼働中`,
      toggleHud: "ゲーム内 RGB オーバーレイを切り替え",
      soundOn: "効果音: オン",
      soundOff: "効果音: ミュート",
      soundToggleAria: "効果音を切り替え",
      languageTitle: "表示言語を切り替え",
      minimize: "最小化",
      minimizeAria: "ウィンドウを最小化",
      restore: "元に戻す",
      maximize: "最大化",
      restoreAria: "ウィンドウを元に戻す",
      maximizeAria: "ウィンドウを最大化",
      closeToTray: "システムトレイに最小化（グローバルホットキー維持）",
      closeWindow: "Nekod を終了",
      closeWindowAria: "ウィンドウを閉じる",
    },
    sidebar: {
      heading: "ライブラリ",
      addLocalGameTitle: "ローカルゲームを追加（フォルダ / .exe）",
      addLocalGameAria: "ローカルゲームを追加",
      rescanTitle: "ゲームディレクトリを再スキャン",
      rescanAria: "再スキャン",
      searchPlaceholder: "ゲームを検索...",
      filterAria: "ゲーム絞り込み",
      filterAll: "すべて",
      filterFavorites: "お気に",
      filterRunning: "起動中",
      gamesHeader: (n) => `ゲーム (${n})`,
      injectedMany: (n) => `注入中 (${n})`,
      injectedOne: "注入中",
      emptyFilter: "該当するゲームはありません。",
      unfavorite: "お気に入り解除",
      favorite: "上部に固定",
    },
    workbench: {
      selectLeftPrompt:
        "左のリストからゲームを選択するか、Mod ストアからパッケージを導入してください。",
      openStoreBtn: "Mod ストアを開く",
      unfavorite: "お気に入り解除",
      favorite: "上部に固定",
      confirmRemove: "削除確定",
      cancel: "キャンセル",
      cannotRemoveWhileInjected: "削除する前に Mod の注入を解除してください",
      removeCustomTitle: "ライブラリから登録を解除（ローカルファイルは残ります）",
      removeCustomBtn: "登録解除",
      bridgeDeployedBadge: (n) => `ブリッジ配備済 (${n})`,
      antiCheatPrefix: "アンチチート",
      copyPathTitle: "クリックしてインストール先パスをコピー",
      playAndInjectTitle: "ゲームを起動し、プロセス検出後に自動注入します",
      launching: "起動中...",
      playAndInject: "起動して自動注入",
      oneClickInjectTitle: "起動中のゲームプロセスに接続して Mod を有効化",
      injecting: "注入中...",
      oneClickInject: "ワンクリック注入",
      stackInjectTitle: "同じゲーム上の別 Mod と併用して重ねて注入",
      stackInject: "追加注入",
      uninjectCurrentTitle: "選択中の Mod のみ解除してロールバック",
      uninjecting: "解除中...",
      uninjectCurrent: "この Mod を解除",
      uninjectSingle: "注入解除",
      uninjectAllTitle: "LIFO 順ですべての注入済み Mod を安全にロールバック",
      uninjectAll: (n) => `すべて解除 (${n})`,
      updateBridgeInjectedTitle: "注入を解除しゲームを終了してから更新してください",
      updateBridgeReadyTitle: "新しいコンパニオンブリッジがあります",
      updatingBridge: "更新中...",
      updateBridge: "ブリッジ更新",
      undeployBridgeTitle:
        "DeploymentLedger に従いブリッジファイルをアンインストール",
      uninstallingBridge: "削除中...",
      undeployBridge: "ブリッジ解除",
      deployBridgeTitle: "エンジンを自動判定してブリッジを配備",
      deployingBridge: "配備中...",
      deployBridge: "ブリッジ配備",
      autoInjectTitle: "許可済みゲーム起動時に自動でアタッチ",
      autoInject: "自動アタッチ",
      hudOverlayTitle: "ゲーム画面右上に RGB オーバーレイ（英語リスト）を表示",
      hudOverlay: "RGB オーバーレイ",
      soundTitle: "注入・切替時の効果音",
      sound: "効果音",
      hotkeysCount: (n) => `${n} hotkeys`,
      noProcessHint:
        "プロセス未検出（「起動して自動注入」またはゲームを手動起動）",
      scanProcessesTitle: "実行中のゲームプロセスを再検索",
      waitingProcessHint: "プロセス待機中...",
      patchesCount: (n) => `patches ${n}`,
      freezesCount: (n) => `freezes ${n}`,
      refreshDiagTitle: "セッション診断を更新",
      otherGameRunning: "別のゲームに注入中です",
      detachOtherSession: "別セッション解除",
      missingUnrealLoader: "Unreal Engine ローダー未導入",
      missingUnityLoader: "Unity ローダー未導入",
      installingLoader: "ダウンロード・検証中...",
      installLoaderBtn: (name) => `${name} を導入`,
      localNmodFound: "ローカル .nmod パッケージ検出",
      featuresCount: (n) => `${n} 機能`,
      importing: "導入中...",
      updateToVersion: (v) => `v${v} へ更新`,
      importNmod: ".nmod 導入",
      modFeaturesTitle: "Mod 機能",
      initializing: "初期化中...",
      quickSetupUnity: "汎用 Unity コントロール初期化",
      quickSetupUe4ss: "汎用 UE4SS コントロール初期化",
      noModUnityHint:
        "Unity ゲームを検出しました。.nmod をドロップするか右上の汎用 Unity 初期化（Pause F8 / 速度 / GC）を実行できます。",
      noModUnrealHint:
        "Unreal Engine ゲームを検出しました。RE-UE4SS とブリッジを導入し、汎用初期化または Mod ストアから導入してください。",
      noModGenericHint:
        "Mod が未導入です。下のコミュニティ Mod、上部の Mod ストア、または .nmod のドラッグ＆ドロップから追加できます。",
      exportNmodTitle: "単一 .nmod アーカイブとして書き出し",
      exportingNmod: "書き出し中...",
      exportNmod: ".nmod 書出",
      deletingMod: "削除中...",
      confirmDeleteMod: "削除確定",
      cannotDeleteWhileInjected: "先に注入を解除してください",
      deleteModTitle: "ローカルライブラリからこの Mod を削除",
      deleteMod: "削除",
      permBannerTitle: "注入前の権限確認 (Default-Deny)",
      permBlockedPrefix: "ブロック",
      approveAndInject: "権限を許可して注入",
      approveOnly: "権限のみ許可",
      modConsoleTitle: "Mod コンソール",
      hotkeyConflict: (k) => `キー競合: ${k}`,
      crossModConflict: (k) => `Mod 間キー重複: ${k}`,
      resetDefaultsTitle: "トグルとホットキーを初期化",
      resetBtn: "リセット",
      saveProfileTitle: "現在のホットキーとプリセットを保存",
      saveProfileBtn: "設定保存",
      filterAll: (n) => `すべて (${n})`,
      filterToggle: (n) => `トグル (${n})`,
      filterValue: (n) => `数値 (${n})`,
      filterAction: (n) => `実行 (${n})`,
      featureSearchPlaceholder: "機能名や説明で絞り込み...",
      stackHint:
        "別の Mod が稼働中：上の「追加注入」をクリックすると両方を同時に有効化できます。",
      injectFirstHint:
        "上の「起動して自動注入」または「ワンクリック注入」を実行すると切り替え・ホットキーが有効になります。",
      closeJsonEditorAria: "JSON エディタを閉じる",
      importJsonBtn: "JSON インポート",
      recordHotkeyTitle: "クリック後にキー入力で設定（Esc 解除・Backspace 消去）",
      recordingKey: "キー入力待ち...",
      bindHotkey: "キー設定",
      executing: "実行中...",
      stateOn: "ON",
      stateOff: "OFF",
      triggerNow: "今すぐ実行",
      applyValue: "適用",
      moreOptionsTitle: "詳細オプション・クイック設定",
      detailsToggleTitle: "クリックでインストール先と環境詳細を開閉",
      idleHintShort: "待機中 · 「起動して自動注入」で開始",
      expandDescHint: "クリックで全文表示",
      collapseDescHint: "クリックで折りたたむ",
      menuSectionBridge: "コンパニオンブリッジ",
      menuSectionToggles: "クイック切替",
      menuSectionGame: "ローカル登録",
      bridgeDrawerTitle: "内蔵コンパニオンブリッジ (DeploymentLedger)",
      bridgeStatusDeployed: "deployed",
      bridgeStatusIdle: "idle",
      ue4ssLoaderReady:
        "RE-UE4SS 準備完了。ゲーム起動時に NekodBridge を読み込みます。",
      unityLoaderReady:
        "ローダー準備完了。ゲーム起動時に NekodUnityBridge を読み込みます。",
      loaderMissingFallback:
        "ローダー未導入。DeploymentLedger からワンクリック導入・復元が可能です。",
      installUe4ssBtn: "RE-UE4SS 導入",
      installLoaderSimple: "ローダー導入",
      uninstallingLoader: "削除中...",
      uninstallLoader: "ローダー削除",
      autoDeployOneClick: "自動配備",
      undeployAndRestore: "解除・復元",
      noBridgeNeeded: (engineLabel) =>
        `検出エンジン (${engineLabel}) は専用ブリッジ不要、または下から手動指定できます。`,
      ledgerFiles: (files) => `台帳記録ファイル: ${files}`,
      allBuiltinBridges: (n) => `内蔵ブリッジ一覧 (${n})`,
      manualSourcePlaceholder: "ソースパス（任意）",
      manualTargetPlaceholder: "ゲーム内相対パス",
      manualDeploy: "手動配備",
      permissionsDrawerTitle: (n) => `権限詳細・取り消し (${n})`,
      revokeAuth: "権限を取り消す",
      liveSessionEventsTitle: (sid) => `リアルタイムイベント (${sid})`,
      compatMatrixTitle: (n) => `互換性マトリクス (${n})`,
      noCompatRecords: "互換性検証記録はまだありません。",
      communityModsTitle: (n) => `コミュニティ利用可能 Mod (${n})`,
      syncing: "同期中...",
      syncCommunity: "ソース同期",
      noCommunityMods:
        "同期済みソースに該当 Mod がありません。Mod ストアを確認してください。",
      installing: "導入中...",
      install: "導入",
      installed: "導入済",
      updating: "更新中...",
      revoked: "失効",
    },
    store: {
      title: "オンライン .nmod ストア",
      badge: "Source-First · SHA-256 Verified",
      subtitlePrefix: "公式バケット",
      subtitleMiddle: "を標準購読し、任意のサードパーティ",
      subtitleSuffix: "を追加して SHA-256 検証付きで導入できます。",
      syncingAll: "インデックス同期中...",
      syncAll: "すべて同期",
      updatesAvailable: (n) => `利用可能な更新 (${n})`,
      updatesReconfirmHint: "更新後は権限の再承認が必要です",
      updating: "更新中...",
      updateTo: (v) => `v${v} へ更新`,
      filterAll: "すべて",
      filterNotInstalled: "未導入",
      filterInstalled: "導入済",
      filterUpdates: "更新あり",
      searchPlaceholder: "ゲーム slug / Mod ID / リポジトリで検索...",
      emptyTitle: "オンラインインデックス未取得",
      emptyHintPrefix: "右上の「すべて同期」をクリックして",
      emptyHintSuffix: "を取得してください。",
      verifiedGameVer: (v) => `検証済みビルド: ${v}`,
      installing: "導入中...",
      installNmod: ".nmod 導入",
      installed: "導入済",
      sourcesDrawer: (n) => `登録済みリポジトリソース (${n})`,
      removeRepo: "削除",
      addThirdPartyTitle: "サードパーティリポジトリを追加",
      addThirdPartyDescPrefix: "HTTPS の",
      addThirdPartyDescMiddle: "URL を入力してください。外部ソースは標準で",
      addThirdPartyDescSuffix: "扱いになります。",
      repoIdPlaceholder: "repository-id",
      repoUrlPlaceholder: "https://raw.githubusercontent.com/.../registry.json",
      addRepoBtn: "ソース追加",
    },
    importCenter: {
      backToWorkbench: "ワークベンチへ戻る",
      title: "ローカル導入 & Cheat Engine 静的変換",
      dropHintPill: ".nmod / .zip / .ct をウィンドウにドロップ可能",
      sec0Title: "0. ローカルゲームを追加（非 Steam / ポータブル）",
      sec0DescPrefix: "ゲームフォルダまたは実行ファイル",
      sec0DescSuffix: "を指定するとエンジンを自動判定します。",
      pathPlaceholder: "D:\\Games\\MyGame または D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "フォルダ選択",
      chooseExe: ".exe 選択",
      labelPlaceholder: "表示名（空欄なら自動認識）",
      importing: "追加中...",
      addToLibrary: "ライブラリに追加",
      sec1TitlePrefix: "1. ローカル",
      sec1TitleMid: "パッケージ導入（",
      sec1TitleSuffix: "ファイルまたはフォルダ）",
      sec1DescPrefix: "JSON Schema と SHA-256 を検証します。ウィンドウへの",
      sec1DescSuffix: "ドロップにも対応しています。",
      nmodPlaceholder: "C:\\Mods\\my-game.nmod またはディレクトリパス",
      chooseNmod: ".nmod 選択",
      verifyAndImport: "検証して導入",
      sec2TitlePrefix: "2. Cheat Engine テーブル変換 (",
      sec2TitleSuffix: "-> 宣言的 Mod)",
      sec2Badge: "fail-closed static converter",
      sec2Desc1: "モジュール相対オフセット・ポインタチェーン・純粋な",
      sec2Desc2: "パッチのみを静的変換します。",
      sec2Desc3: "やリモート実行スクリプトは安全のため拒否されます。",
      customTargetOption: "ターゲットゲーム ID / 名称を手動指定...",
      trustUnverifiedOpt: "Unverified（設定で許可が必要）",
      trustCommunityOpt: "Community Verified（自己監査済み）",
      trustOfficialOpt: "Official",
      customGameIdPlaceholder: "ゲーム canonical slug（任意: sample-rpg）",
      customGameNamePlaceholder: "表示名（例: Sample RPG）",
      ctPathPlaceholder: "C:\\Users\\...\\Downloads\\Game.CT",
      chooseCt: ".CT 選択",
      defaultModulePlaceholder: "既定モジュール名（任意: Game.exe）",
      convertingCt: "静的解析中...",
      convertAndImportCt: "変換して導入",
      ctUnverifiedHintPrefix:
        "注意: Default-Deny により Unverified は既定でブロックされます。「設定」で許可するか監査後に",
      ctUnverifiedHintSuffix: "を選択してください。",
      openInWorkbench: "ワークベンチで確認",
      convertedFeatures: (n) => `変換済み機能 (${n})`,
      skippedEntries: (n) => `拒否 / スキップ項目 (${n})`,
      unnamedEntry: "(無名項目)",
      sec3Title: "3. Game Manifest JSON を登録",
      gameManifestPlaceholder: "検証済み game-manifest.json を貼り付け...",
      verifyAndRegisterGame: "検証して登録",
    },
    settings: {
      backToWorkbench: "ワークベンチへ戻る",
      title: "環境設定とセキュリティポリシー",
      languageSection: "表示言語 / Language",
      languageLabel: "インターフェース言語",
      languageDesc:
        "アプリ全体のラベル・メニュー・エラー通知を即時切り替えます（中文 / English / 日本語）。",
      desktopBehavior: "デスクトップ動作とタスクトレイ",
      minimizeToTrayTitle: "閉じるボタンでタスクトレイに最小化（推奨）",
      minimizeToTrayDesc:
        "グローバルホットキーと HUD オーバーレイを維持します。トレイ右クリックで完全終了できます。",
      autoAttachTitle: "許可済み Mod の自動アタッチ",
      autoAttachDesc:
        "対象ゲームのプロセス起動を検出すると自動で注入します（事前承認済み Mod のみ）。",
      hudOverlayTitle: "ゲーム内 RGB オーバーレイ (HUD ArrayList)",
      hudOverlayDesc:
        "最前面にクリックスルー表示します（ウィンドウ / ボーダレスモード推奨）。",
      soundTitle: "注入・トグル効果音を有効化",
      soundDesc: "ホットキー操作や注入完了時に軽快な通知音を再生します。",
      securitySection: "Default-Deny セキュリティ設定",
      allowUnverifiedTitle: "未検証 (Unverified) ソースの宣言的 Mod を許可",
      allowUnverifiedDesc:
        "サードパーティ未検証 Mod の手動権限承認を可能にします（SHA-256 検証は維持）。",
      allowNativeTitle: "サードパーティ製ネイティブモジュール許可（高リスク・既定オフ）",
      allowNativeDesc: "ローカルコードを完全に信頼できる場合のみ有効化してください。",
      customFoldersTitle: "カスタムゲームフォルダ / ローカル実行ファイル",
      pathPlaceholder: "D:\\Games\\MyGame または D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "フォルダ選択",
      chooseExe: ".exe 選択",
      labelPlaceholder: "表示名（空欄なら自動認識）",
      addFolderBtn: "追加",
      removeBtn: "削除",
      bridgesMatrixTitle: (n) => `内蔵コンパニオンブリッジ一覧 (${n})`,
    },
    logs: {
      backToWorkbench: "ワークベンチへ戻る",
      title: "診断・監査ログ",
      exportBundle: "診断バンドル書出",
      refresh: "更新",
      closeBundleAria: "バンドル表示を閉じる",
      emptyLogs: "診断イベントはまだありません。",
    },
    modals: {
      addGameTitle: "ローカルゲームを追加",
      closeAria: "ダイアログを閉じる",
      addGameDescPrefix:
        "Steam / GOG / Epic のゲームは自動検出されます。ポータブル版などはフォルダまたは",
      pathPlaceholder: "D:\\Games\\MyGame または D:\\Games\\MyGame\\Game.exe",
      chooseFolder: "フォルダ選択",
      chooseExe: ".exe 選択",
      labelPlaceholder: "表示名（空欄なら自動認識）",
      importing: "追加中...",
      addToLibrary: "ライブラリに追加",
      dropDefaultPreview: ".nmod / .zip / .ct / フォルダ",
      dropTitle: "ドロップして検証・インポート",
      dropDesc1: "単一ファイル",
      dropDesc2: "、Mod ディレクトリ、または Cheat Engine",
      dropDesc3: "に対応しています",
    },
    app: {
      browserPreviewBanner:
        "ブラウザプレビューモード：モック IPC データを表示中。実プロセス注入とグローバルホットキーは Tauri デスクトップ版で動作します。",
      closeErrorAria: "エラーを閉じる",
      closeStatusAria: "ステータスを閉じる",
      copiedPathToast: "インストールパスをコピーしました",
      installPathStatus: (p) => `インストールパス: ${p}`,
      autoAttachOn: "自動アタッチ有効：対象ゲーム起動時に自動注入します",
      autoAttachOff: "自動アタッチを無効化しました",
      hudOverlayOn: "RGB HUD オーバーレイを有効化しました",
      hudOverlayOff: "RGB HUD オーバーレイを無効化しました",
      trayOn: "閉じる操作でシステムトレイへ格納します",
      trayOff: "閉じる操作で Nekod を終了します",
      rescanSummary: (n) => `再スキャン完了：${n} 本のゲームを検出しました。`,
      ctDropToast: ".CT を読み込みました。「インポート / 変換」で対象を確認してください",
      dropImportOkToast: "SHA-256 検証を完了しローカル Mod を導入しました",
      dropImportStatus: (p) => `ドロップされたパッケージを導入しました：${p}`,
      antiCheatRejectInject:
        "アンチチート保護が検出されたため注入を拒否しました（Nekod はオフラインシングルプレイ専用です）。",
      noProcessError:
        "実行中のゲームプロセスが見つかりません。ゲームを起動するか「起動して自動注入」を使用してください。",
      autoInjectSuccess: (n, p, pid) =>
        `自動注入完了「${n}」 -> ${p} (PID ${pid})`,
      injectSuccess: (n, p, pid) => `注入完了「${n}」 -> ${p} (PID ${pid})`,
      waitingBridgeReady: (p, pid) =>
        `プロセス検出 (${p} · PID ${pid})、コンパニオンブリッジ待機中...`,
      otherGameSessionActive:
        "別のゲームセッションが稼働中です。切り替える前に注入を解除してください。",
      trainerAlreadyRunning: (n) => `「${n}」は既に現在のゲームで稼働中です。`,
      trainerBlocked: (r) => `セキュリティポリシーによりブロックされました：${r}`,
      requiresConfirmationStatus: (n) =>
        `「${n}」の実行権限承認が必要です。下の「権限を許可して注入」を押してください。`,
      antiCheatRejectLaunch:
        "アンチチート検出のため「起動して自動注入」は無効です。",
      bridgeAutoUpdatedToast: "ゲーム起動前にコンパニオンブリッジを更新しました",
      launchedNeedUnityStarter: (t) =>
        `起動しました (${t})。「汎用 Unity コントロール初期化」または Mod を導入してください。`,
      launchedNeedUe4ssStarter: (t) =>
        `起動しました (${t})。「汎用 UE4SS コントロール初期化」または Mod を導入してください。`,
      launchedNeedMod: (t) =>
        `起動しました (${t})。先にローカル .nmod を導入してください。`,
      launchedWaitingInject: (t) =>
        `起動しました (${t})。プロセス待機後に自動注入します...`,
      permissionsApprovedStatus: (n, v) =>
        `${n} v${v} の宣言権限を承認しました。`,
      uninjectSingleStatus: (n, c, f) =>
        `「${n}」を LIFO 順でロールバック解除しました（競合: ${c} / 失敗: ${f}）`,
      uninjectMultiStatus: (cnt, c, f) =>
        `${cnt} 個の Mod を LIFO 順で解除しました（競合: ${c} / 失敗: ${f}）`,
      gameExitedStatus: (t) =>
        `「${t}」の終了を検出し、注入セッションを切断しました。`,
      bridgeAutoDeployed: (e, files) =>
        `${e} を自動判定しブリッジを配備しました (${files})`,
      manualBridgeInputError:
        "ソースパスとゲーム内相対パスを入力するか、「自動配備」を使用してください。",
      bridgeManualDeployed: (target) =>
        `DeploymentLedger 経由で ${target} に配備しました。`,
      bridgeUndeployed: (rem, res, con) =>
        `ブリッジ解除：${rem} 削除、${res} 復元（競合: ${con}）`,
      loaderInitProgress: "非同期ダウンロードと検証を初期化中...",
      loaderInstalledNeedBridge: (l) =>
        `${l} を導入しました。次に「ブリッジ配備」を実行してゲームを再起動してください。`,
      loaderInstalledNeedRestart: (l) =>
        `${l} を導入しました。ゲームを起動中の場合は再起動してください。`,
      loaderInstalledToast: (s) => `${s} ローダー導入完了（再起動で有効）`,
      loaderUndeployed: (s, rem, res, con) =>
        `${s} ローダー削除：${rem} 削除、${res} 復元（競合: ${con}）`,
      unityStarterDoneStatus: (tid) =>
        `汎用 Unity Mod (${tid}) を初期化しました。権限承認後に注入してください。`,
      unityStarterDoneToast: "汎用 Unity Mod 初期化完了",
      ue4ssStarterDoneStatus: (tid) =>
        `汎用 UE4SS Mod (${tid}) を初期化しました。権限承認後に注入してください。`,
      ue4ssStarterDoneToast: "汎用 UE4SS Mod 初期化完了",
      nmodInstalledStatus: (n, c) =>
        `.nmod「${n}」(${c} 機能) を導入しました。権限承認後に注入できます。`,
      nmodInstalledToast: (n) => `.nmod 導入完了: ${n}`,
      invalidNumberError: (n) => `「${n}」に有効な数値を入力してください。`,
      featureToggledOn: (n, s) => `${n} ON${s}`,
      featureToggledOff: (n, s) => `${n} OFF${s}`,
      featureTriggered: (n, s) => `実行: ${n}${s}`,
      permissionsRevoked: (n) => `「${n}」の権限を取り消しました。`,
      customGameImported: (t) => `ローカルゲーム「${t}」を追加しました。`,
      customGameAdded: "カスタムゲームパスを追加してスキャンしました。",
      customGameRemoved: "ライブラリからカスタム登録を解除しました（実ファイルは維持）。",
      uninstallStopFirstError: "削除する前に「注入解除」で停止してください。",
      trainerUninstalled: (n) => `「${n}」を削除しました（権限も取り消し済み）。`,
      nmodExportedStatus: (p) => `.nmod を書き出しました：${p}`,
      nmodExportedToast: ".nmod 書出完了",
      localTrainerIngested: "ローカルパッケージを検証・導入しました。",
      gameManifestRegistered: (gid) => `ゲームマニフェスト '${gid}' を登録しました。`,
      ctImportedToast: (n) => `.CT から ${n} 機能を変換・導入しました`,
      ctSkippedToast: (n) => `.CT 導入拒否：${n} 個の非宣言的項目をブロック`,
      syncComplete: (chk, upd, unch, fail, rev) =>
        `同期完了（${chk} ソース）：${upd} 更新 / ${unch} 変更なし / ${fail} 失敗 / ${rev} 失効`,
      repoAdded: "サードパーティソースを追加しました。同期ボタンで取得してください。",
      repoRemoved: (id) => `ソース '${id}' を削除しました。`,
      communityInstalledStatus: (tid) =>
        `SHA-256 検証済み '${tid}' を導入しました。注入前に権限承認を行ってください。`,
      communityInstalledToast: (tid) => `オンライン Mod 導入: ${tid}`,
      hotkeyBoundToast: (hk) => `ホットキー設定: ${hk}`,
      hotkeyClearedToast: "ホットキーをクリアしました",
      profileSavedStatus: (n) => `「${n}」のホットキー設定を保存しました。`,
      profileSavedToast: "プロファイル保存完了",
      profileResetStatus: (n) => `「${n}」のデフォルト設定に戻しました。`,
      presetAppliedStatus: (p) => `プリセット「${p}」を適用・保存しました。`,
      profileExportedStatus: (n) => `「${n}」の JSON 設定を書き出しました。`,
      profileImportedStatus: (tid) => `'${tid}' の JSON 設定を読み込みました。`,
      bundleGeneratedStatus: "監査可能な診断バンドル JSON を生成しました。",
      unknownEngine: "不明",
      pickFolderTitle: "ゲームルートフォルダを選択",
      pickExeTitle: "ゲーム実行ファイル (.exe) を選択",
      pickNmodTitle: ".nmod アーカイブを選択",
      pickNmodFolderTitle: "展開済み Mod フォルダを選択",
      pickCtTitle: "Cheat Engine .CT ファイルを選択",
    },
  },
};

const PERMISSIONS_BY_LOCALE: Record<Locale, Record<PermissionScope, string>> = {
  "zh-CN": {
    "process.discover": "发现运行中的游戏进程",
    "process.read_memory": "读取游戏内存数值",
    "process.write_memory": "写入游戏内存以启用模组功能",
    "process.allocate_memory": "在游戏进程中分配内存页",
    "process.suspend_resume": "打补丁时短暂停顿游戏线程",
    "filesystem.read_save_dir": "读取游戏存档目录",
    "filesystem.write_save_dir": "修改游戏存档目录文件",
    "runtime.native_module": "加载原生运行时模块（默认禁用）",
  },
  "en-US": {
    "process.discover": "Discover running game processes",
    "process.read_memory": "Read game memory values",
    "process.write_memory": "Write game memory to toggle features",
    "process.allocate_memory": "Allocate memory pages inside game process",
    "process.suspend_resume": "Briefly suspend game threads while patching",
    "filesystem.read_save_dir": "Read game save directory",
    "filesystem.write_save_dir": "Modify files in game save directory",
    "runtime.native_module": "Load native runtime module (off by default)",
  },
  "ja-JP": {
    "process.discover": "実行中のゲームプロセスを検出",
    "process.read_memory": "ゲームメモリ値を読み取り",
    "process.write_memory": "機能有効化のためゲームメモリへ書き込み",
    "process.allocate_memory": "ゲームプロセス内にメモリを確保",
    "process.suspend_resume": "パッチ適用時にゲームスレッドを一時停止",
    "filesystem.read_save_dir": "セーブフォルダを読み取り",
    "filesystem.write_save_dir": "セーブフォルダのファイルを変更",
    "runtime.native_module": "ネイティブモジュール読み込み（既定オフ）",
  },
};

export function translatePermission(
  scope: PermissionScope,
  locale: Locale,
): string {
  return PERMISSIONS_BY_LOCALE[locale][scope] ?? scope;
}

export function translateTrust(tier: TrustTier, locale: Locale): string {
  if (locale === "ja-JP") {
    if (tier === "official") return "公式";
    if (tier === "community_verified") return "検証済";
    return "未検証";
  }
  if (locale === "zh-CN") {
    if (tier === "official") return "Official";
    if (tier === "community_verified") return "Verified";
    return "Unverified";
  }
  if (tier === "official") return "Official";
  if (tier === "community_verified") return "Verified";
  return "Unverified";
}

export function translateRuntimeError(raw: string, locale: Locale): string {
  const lower = raw.toLowerCase();
  if (
    lower.includes("main thread dispatch timeout") ||
    lower.includes("bridge read timed out") ||
    lower.includes("game main thread is busy")
  ) {
    if (locale === "en-US") {
      return "Game main thread is loading or paused. Update the companion bridge and retry once in-game.";
    }
    if (locale === "ja-JP") {
      return "ゲームのメインスレッドが読み込み中または停止中です。ブリッジを更新し、ゲーム画面に入ってから再試行してください。";
    }
    return "游戏当前处于加载中或主线程暂停状态（切换场景/加载画面/旧版桥接未开启后台运行），请先更新伴生桥接并等进入游戏画面后再操作。";
  }

  const noInstanceMatch = raw.match(
    /no live instance of '([^']+)' found in active scene/i,
  );
  if (noInstanceMatch) {
    const fullCls = noInstanceMatch[1];
    const shortCls = fullCls.split(".").pop() ?? fullCls;
    if (locale === "en-US") {
      return `Target object "${shortCls}" is not loaded in the current scene yet (enter gameplay first).`;
    }
    if (locale === "ja-JP") {
      return `対象オブジェクト「${shortCls}」が現在のシーンにまだ存在しません（プレイ画面へ進んでからお試しください）。`;
    }
    return `当前场景尚未加载目标对象「${shortCls}」（请先进入实际游玩关卡，不要在标题/加载界面开启此功能）。`;
  }

  if (lower.includes("not in an active combat stage")) {
    if (locale === "en-US") {
      return "Not currently in an active combat stage; enter a combat wave first.";
    }
    if (locale === "ja-JP") {
      return "戦闘ステージ外です。戦闘ウェーブに入ってから実行してください。";
    }
    return "当前不在战斗关卡中，请进入战斗/波次关卡后再点击「一键通关」。";
  }

  if (lower.includes("no active savegame found")) {
    if (locale === "en-US") {
      return "No active savegame loaded; enter the hub or a stage first.";
    }
    if (locale === "ja-JP") {
      return "セーブデータが読み込まれていません。拠点またはステージへ進んでください。";
    }
    return "尚未加载存档，请先进入游戏主城或战斗关卡后再点击「仓库一键满仓库」。";
  }

  if (
    lower.includes("unsupported ue4ss bridge op") ||
    lower.includes("fill_items") ||
    lower.includes("trigger_goal") ||
    lower.includes("persist_uproperty") ||
    lower.includes("add_uproperty") ||
    lower.includes("set_target_lock") ||
    lower.includes("set_auto_fire") ||
    lower.includes("set_spinbot") ||
    lower.includes("set_damage_aura") ||
    lower.includes("pin_field") ||
    lower.includes("pin_target_lock") ||
    lower.includes("pin_damage_aura") ||
    lower.includes("pin_spinbot") ||
    lower.includes("set_gameobject_active") ||
    lower.includes("copy_field") ||
    lower.includes("invoke_method_args") ||
    lower.includes("replace_texture_material") ||
    lower.includes("restore_texture_material") ||
    lower.includes("pin_method_hook") ||
    lower.includes("unpin_method_hook") ||
    lower.includes("outdated") ||
    lower.includes("v0.3.0") ||
    lower.includes("v0.4.0") ||
    lower.includes("v0.5.0") ||
    lower.includes("v0.5.1") ||
    lower.includes("v0.5.2") ||
    lower.includes("v0.5.3") ||
    lower.includes("v0.5.4") ||
    lower.includes("v0.5.5") ||
    lower.includes("v0.6.0") ||
    lower.includes("v0.7.0") ||
    lower.includes("v0.7.1") ||
    lower.includes("v0.7.2") ||
    lower.includes("v0.7.3")
  ) {
    if (locale === "en-US") {
      return "Companion bridge is outdated. Close the game and click Update Bridge (or Play & Inject) to upgrade to Unity v0.5.5 / UE4SS v0.7.3.";
    }
    if (locale === "ja-JP") {
      return "コンパニオンブリッジのバージョンが古いです。ゲームを終了し「ブリッジ更新」で最新版（Unity v0.5.5 / UE4SS v0.7.3）へ更新してください。";
    }
    return "伴生桥接版本过旧，请关闭游戏后点击右上角「更新伴生桥接」（或「启动游戏并注入」自动更新）升级到最新版（Unity v0.5.5 / UE4SS v0.7.3）。";
  }

  if (
    lower.includes("integrity mismatch") ||
    lower.includes("checksum") ||
    lower.includes("sha-256") ||
    lower.includes("sha256")
  ) {
    if (locale === "en-US") {
      return "Local mod files changed; click Update to re-verify SHA-256 and confirm permissions.";
    }
    if (locale === "ja-JP") {
      return "ローカル Mod ファイルが変更されています。再インポートして SHA-256 と権限を確認してください。";
    }
    return "本地模组文件已更新，请先点击上方的「更新至新版本」重新导入并确认运行权限。";
  }

  if (
    lower.includes("bridge connect") ||
    lower.includes("10061") ||
    lower.includes("refused")
  ) {
    if (locale === "en-US") {
      return "Cannot connect to companion bridge (game may have exited or bridge is still initializing).";
    }
    if (locale === "ja-JP") {
      return "コンパニオンブリッジに接続できません（ゲーム終了済み、または起動初期化中です）。";
    }
    return "无法连接到游戏伴生桥接（游戏可能已退出或桥接尚未加载完成）。";
  }

  return raw;
}

interface I18nContextValue {
  locale: Locale;
  setLocale: (next: Locale) => void;
  cycleLocale: () => void;
  m: TranslationMessages;
}

const I18nContext = createContext<I18nContextValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(() => detectInitialLocale());

  const setLocale = useCallback((next: Locale) => {
    setLocaleState(next);
    try {
      localStorage.setItem(LOCALE_STORAGE_KEY, next);
    } catch {
      // Ignore storage quota errors
    }
  }, []);

  const cycleLocale = useCallback(() => {
    const order: Locale[] = ["zh-CN", "en-US", "ja-JP"];
    const next = order[(order.indexOf(locale) + 1) % order.length];
    setLocale(next);
  }, [locale, setLocale]);

  useEffect(() => {
    if (typeof document !== "undefined") {
      document.documentElement.lang = locale;
    }
  }, [locale]);

  const value = useMemo<I18nContextValue>(
    () => ({
      locale,
      setLocale,
      cycleLocale,
      m: MESSAGES[locale],
    }),
    [locale, setLocale, cycleLocale],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const ctx = useContext(I18nContext);
  if (!ctx) {
    throw new Error("useI18n must be used within <I18nProvider>");
  }
  return ctx;
}
