/**
 * Refactoring UI Constraint Brief (Workflow B & C):
 * - Content Ranking:
 *   - Primary: Main page canvas (<Outlet />) and current page header title
 *   - Secondary: Grouped sidebar navigation items (active state uses left indicator bar + soft tint per §2.4, never competing with page primary CTAs)
 *   - Tertiary: Group section labels, theme/version metadata, GitHub link, and de-emphasized logout (§2.8.1)
 * - Grouping (§3.6):
 *   - Three domain groups ("监控与调试", "网关核心", "系统运维") with outer space-y-5 > inner space-y-0.5
 * - Tokens:
 *   - Surfaces: bg-background, bg-card, bg-muted, border-border
 *   - Text tiers (max 3): text-foreground, text-muted-foreground, text-muted-foreground/75
 */
import { useState, useEffect } from 'react';
import { Outlet, Link, useLocation } from 'react-router-dom';
import {
  LayoutDashboard,
  Server,
  FlaskConical,
  KeyRound,
  Settings as SettingsIcon,
  LogOut,
  ScrollText,
  TerminalSquare,
  Puzzle,
  Sun,
  Moon,
  Laptop,
  Menu,
  X,
  FolderOpen,
  Github,
  Loader2,
  ArrowUpCircle,
  Download,
  ExternalLink,
  Copy,
  GitBranch,
  Palette,
  PanelLeftClose,
  PanelLeftOpen,
} from 'lucide-react';
import { useAuthStore } from '../store/authStore';
import {
  PALETTE_PRESETS,
  STYLE_PRESETS,
  Theme,
  useThemeStore,
} from '../store/themeStore';
import { apiFetch } from '../lib/api';
import { toastSuccess, toastError } from './Toast';
import ThemeCustomizer from './ThemeCustomizer';
import * as Dialog from '@radix-ui/react-dialog';

interface NavItem {
  id: string;
  label: string;
  icon: typeof LayoutDashboard;
}

interface NavGroup {
  groupLabel: string;
  items: NavItem[];
}

const navGroups: NavGroup[] = [
  {
    groupLabel: '监控与调试',
    items: [
      { id: '/', label: '仪表盘', icon: LayoutDashboard },
      { id: '/playground', label: '测试工坊', icon: FlaskConical },
      { id: '/logs', label: '系统日志', icon: ScrollText },
      { id: '/backend-logs', label: '后台日志', icon: TerminalSquare },
    ],
  },
  {
    groupLabel: '网关核心',
    items: [
      { id: '/channels', label: '渠道配置', icon: Server },
      { id: '/admin', label: '密钥管理', icon: KeyRound },
      { id: '/plugins', label: '插件管理', icon: Puzzle },
    ],
  },
  {
    groupLabel: '系统运维',
    items: [
      { id: '/workspace', label: '工作区', icon: FolderOpen },
      { id: '/settings', label: '系统设置', icon: SettingsIcon },
    ],
  },
];

const allNavItems: NavItem[] = navGroups.flatMap((g) => g.items);

function NavContent({
  pathname,
  collapsed = false,
  theme,
  setTheme,
  onOpenCustomizer,
  activeSwatch,
  activePaletteName,
  logout,
  onNavClick,
  versionLabel,
  checkingUpdate,
  onCheckUpdate,
  hasUpdate,
}: {
  pathname: string;
  collapsed?: boolean;
  theme: Theme;
  setTheme: (t: Theme) => void;
  onOpenCustomizer: () => void;
  activeSwatch: string;
  activePaletteName: string;
  logout: () => void;
  onNavClick: () => void;
  versionLabel: string;
  checkingUpdate: boolean;
  onCheckUpdate: () => void;
  hasUpdate: boolean;
}) {
  return (
    <>
      <nav
        className={`flex-1 overflow-y-auto no-scrollbar ${
          collapsed ? 'px-2.5 py-4 space-y-4' : 'px-3 py-4 space-y-5'
        }`}
      >
        {navGroups.map((group, gi) => (
          <div key={group.groupLabel} className="space-y-0.5">
            {collapsed ? (
              gi > 0 && <div className="mx-2 my-2 border-t border-border/70" />
            ) : (
              <div className="px-3 pb-1 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/75 select-none">
                {group.groupLabel}
              </div>
            )}

            {group.items.map((item) => {
              const Icon = item.icon;
              const isActive = pathname === item.id;
              return (
                <Link
                  key={item.id}
                  to={item.id}
                  onClick={onNavClick}
                  title={collapsed ? item.label : undefined}
                  className={`group relative flex items-center ${
                    collapsed ? 'justify-center px-0 py-2.5' : 'gap-3 px-3 py-2'
                  } rounded-lg text-sm transition-colors ${
                    isActive
                      ? 'bg-primary/[0.11] text-foreground font-semibold'
                      : 'text-muted-foreground font-medium hover:text-foreground hover:bg-muted/70'
                  }`}
                >
                  {isActive && (
                    <span className="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-full bg-primary" />
                  )}
                  <Icon
                    className={`w-[18px] h-[18px] flex-shrink-0 transition-colors ${
                      isActive
                        ? 'text-primary'
                        : 'text-muted-foreground group-hover:text-foreground'
                    }`}
                  />
                  {!collapsed && <span className="truncate">{item.label}</span>}
                </Link>
              );
            })}
          </div>
        ))}
      </nav>

      <div
        className={`border-t border-border ${
          collapsed ? 'p-2.5 space-y-1.5' : 'p-3 space-y-1.5'
        }`}
      >
        {/* Appearance Studio Trigger */}
        <button
          type="button"
          onClick={onOpenCustomizer}
          title={collapsed ? `外观与主题 (${activePaletteName})` : undefined}
          className={`w-full flex items-center ${
            collapsed ? 'justify-center p-2' : 'justify-between px-3 py-2'
          } rounded-lg border border-border/80 bg-muted/45 hover:bg-muted text-xs font-medium text-foreground transition-colors`}
        >
          <span className="flex items-center gap-2 min-w-0">
            <Palette className="w-4 h-4 text-primary flex-shrink-0" />
            {!collapsed && <span className="truncate">外观与主题</span>}
          </span>
          {!collapsed && (
            <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground flex-shrink-0">
              <span
                className="w-2.5 h-2.5 rounded-full border border-black/15 dark:border-white/25"
                style={{ backgroundColor: activeSwatch }}
              />
              {activePaletteName}
            </span>
          )}
        </button>

        {/* Quick Mode Switcher */}
        {!collapsed ? (
          <div className="flex items-center bg-muted/70 p-0.5 rounded-lg border border-border/50">
            <button
              type="button"
              onClick={() => setTheme('light')}
              title="浅色模式"
              className={`flex-1 flex justify-center py-1.5 rounded-md text-xs font-medium transition-colors ${
                theme === 'light'
                  ? 'bg-card text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              <Sun className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={() => setTheme('system')}
              title="跟随系统"
              className={`flex-1 flex justify-center py-1.5 rounded-md text-xs font-medium transition-colors ${
                theme === 'system'
                  ? 'bg-card text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              <Laptop className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={() => setTheme('dark')}
              title="深色模式"
              className={`flex-1 flex justify-center py-1.5 rounded-md text-xs font-medium transition-colors ${
                theme === 'dark'
                  ? 'bg-card text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              <Moon className="w-3.5 h-3.5" />
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
            title="切换浅色 / 深色"
            className="w-full flex justify-center p-2 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
          >
            {theme === 'light' ? <Sun className="w-4 h-4" /> : <Moon className="w-4 h-4" />}
          </button>
        )}

        {/* Meta row: GitHub + Version + Logout */}
        {!collapsed ? (
          <div className="pt-1 space-y-0.5">
            <div className="flex items-center justify-between px-2 py-1 text-xs text-muted-foreground">
              <a
                href="https://github.com/Mitasul/miora"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 hover:text-foreground transition-colors"
              >
                <Github className="w-3.5 h-3.5" />
                <span>GitHub</span>
              </a>

              {versionLabel && (
                <button
                  type="button"
                  onClick={onCheckUpdate}
                  disabled={checkingUpdate}
                  className="inline-flex items-center gap-1 font-mono text-[11px] text-muted-foreground hover:text-foreground transition-colors relative"
                  title={hasUpdate ? '有新版本可用，点击更新' : '点击检查更新'}
                >
                  {checkingUpdate ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : (
                    <GitBranch className="w-3 h-3" />
                  )}
                  <span className="truncate max-w-[110px]">{versionLabel}</span>
                  {hasUpdate && (
                    <span className="w-2 h-2 bg-destructive rounded-full animate-pulse ml-0.5" />
                  )}
                </button>
              )}
            </div>

            <button
              type="button"
              onClick={logout}
              className="flex items-center gap-2.5 px-3 py-2 rounded-lg text-xs font-medium text-muted-foreground hover:text-destructive hover:bg-destructive/10 w-full transition-colors"
            >
              <LogOut className="w-4 h-4" />
              退出登录
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={logout}
            title="退出登录"
            className="w-full flex justify-center p-2 rounded-lg text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-colors"
          >
            <LogOut className="w-4 h-4" />
          </button>
        )}
      </div>
    </>
  );
}

export default function Layout() {
  const { token, logout } = useAuthStore();
  const {
    theme,
    style,
    palette,
    sidebarCollapsed,
    setTheme,
    toggleSidebarCollapsed,
  } = useThemeStore();
  const location = useLocation();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const [customizerOpen, setCustomizerOpen] = useState(false);

  // Version & Update
  const [currentVersion, setCurrentVersion] = useState('');
  const [deployType, setDeployType] = useState('');
  const [gitInfo, setGitInfo] = useState<{ commit?: string }>({});
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [showUpdateDialog, setShowUpdateDialog] = useState(false);
  const [hasUpdate, setHasUpdate] = useState(false);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const [updateInfo, setUpdateInfo] = useState<any>(null);

  const isUpdateDismissed = (latestVersion: string): boolean => {
    const skipped = localStorage.getItem('zoaholic_skip_version');
    if (skipped === latestVersion) return true;
    const snoozeUntil = localStorage.getItem('zoaholic_snooze_until');
    if (snoozeUntil && Date.now() < parseInt(snoozeUntil, 10)) return true;
    return false;
  };

  useEffect(() => {
    if (!token) return;
    (async () => {
      try {
        const vRes = await apiFetch('/v1/system/version', {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (vRes.ok) {
          const data = await vRes.json();
          setCurrentVersion(data.version || '');
          setDeployType(data.deploy_type || '');
          setGitInfo(data.git || {});
        }
        const uRes = await apiFetch('/v1/system/check-update', {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (uRes.ok) {
          const data = await uRes.json();
          setUpdateInfo(data);
          const latestVer =
            data.latest_release?.version ||
            data.latest_pypi?.version ||
            data.latest_docker?.version ||
            '';
          setHasUpdate(data.has_update && !isUpdateDismissed(latestVer));
        }
      } catch {
        // Silent background check
      }
    })();
  }, [token]);

  const handleSkipVersion = () => {
    const ver = updateInfo?.latest_release?.version || updateInfo?.latest_pypi?.version || '';
    if (ver) localStorage.setItem('zoaholic_skip_version', ver);
    setHasUpdate(false);
    setShowUpdateDialog(false);
    toastSuccess('已跳过此版本');
  };

  const handleSnooze = () => {
    localStorage.setItem('zoaholic_snooze_until', String(Date.now() + 24 * 60 * 60 * 1000));
    setHasUpdate(false);
    setShowUpdateDialog(false);
    toastSuccess('24 小时内不再提醒');
  };

  const handleCheckUpdate = async () => {
    if (!token) return;
    if (updateInfo?.has_update) {
      setShowUpdateDialog(true);
      return;
    }
    setCheckingUpdate(true);
    try {
      const res = await apiFetch('/v1/system/check-update', {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (res.ok) {
        const data = await res.json();
        setUpdateInfo(data);
        setHasUpdate(data.has_update || false);
        if (data.has_update) {
          setShowUpdateDialog(true);
        } else {
          toastSuccess('已是最新版本');
        }
      } else {
        toastError(await res.text(), '检查更新失败');
      }
    } catch (err) {
      toastError(err, '检查更新失败');
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handlePerformUpdate = async () => {
    if (!token) return;
    setUpdating(true);
    try {
      const res = await apiFetch('/v1/system/update', {
        method: 'POST',
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      });
      if (res.ok) {
        const data = await res.json();
        toastSuccess(`更新到 v${data.new_version}，服务即将重启...`);
        setShowUpdateDialog(false);
        setTimeout(() => window.location.reload(), 5000);
      } else {
        toastError(await res.text(), '更新失败');
      }
    } catch (err) {
      toastError(err, '更新失败');
    } finally {
      setUpdating(false);
    }
  };

  const versionLabel = currentVersion
    ? `v${currentVersion}${
        deployType === 'git' && gitInfo.commit
          ? ` (${gitInfo.commit})`
          : deployType === 'docker'
          ? ' · docker'
          : deployType === 'pip'
          ? ' · pip'
          : ''
      }`
    : '';

  const activePaletteMeta =
    PALETTE_PRESETS.find((p) => p.id === palette) || PALETTE_PRESETS[0];
  const activeStyleMeta =
    STYLE_PRESETS.find((s) => s.id === style) || STYLE_PRESETS[0];

  const currentGroup = navGroups.find((g) =>
    g.items.some((i) => i.id === location.pathname)
  );
  const currentItem = allNavItems.find((item) => item.id === location.pathname);
  const currentLabel = currentItem?.label || 'Miora';
  const isViewportPage = ['/playground', '/workspace', '/backend-logs', '/logs'].includes(
    location.pathname
  );
  const isPlayground = location.pathname === '/playground';

  const sharedNavProps = {
    pathname: location.pathname,
    theme,
    setTheme,
    onOpenCustomizer: () => {
      setMobileMenuOpen(false);
      setCustomizerOpen(true);
    },
    activeSwatch: activePaletteMeta.swatches[0],
    activePaletteName: activePaletteMeta.name,
    logout,
    onNavClick: () => setMobileMenuOpen(false),
    versionLabel,
    checkingUpdate,
    onCheckUpdate: handleCheckUpdate,
    hasUpdate,
  };

  return (
    <div className="flex h-screen bg-background text-foreground font-sans">
      {/* Desktop Sidebar */}
      <aside
        className={`${
          sidebarCollapsed ? 'w-[68px]' : 'w-60'
        } bg-card border-r border-border flex-col hidden md:flex transition-[width] duration-200 ease-out flex-shrink-0`}
      >
        <div
          className={`h-14 flex items-center ${
            sidebarCollapsed ? 'justify-center px-2' : 'justify-between px-4'
          } border-b border-border`}
        >
          {!sidebarCollapsed && (
            <Link to="/" className="flex items-center gap-2.5 min-w-0">
              <img
                src="/miora.png"
                alt="Miora"
                className="w-7 h-7 rounded-lg shadow-sm flex-shrink-0"
              />
              <div className="min-w-0">
                <span className="font-bold text-base tracking-tight block leading-none">
                  Miora
                </span>
                <span className="text-[10px] font-mono text-muted-foreground">
                  lite gateway
                </span>
              </div>
            </Link>
          )}
          <button
            type="button"
            onClick={toggleSidebarCollapsed}
            title={sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'}
            className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
          >
            {sidebarCollapsed ? (
              <PanelLeftOpen className="w-4 h-4" />
            ) : (
              <PanelLeftClose className="w-4 h-4" />
            )}
          </button>
        </div>

        <NavContent {...sharedNavProps} collapsed={sidebarCollapsed} />
      </aside>

      {/* Mobile Menu Overlay */}
      {mobileMenuOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden"
          onClick={() => setMobileMenuOpen(false)}
        />
      )}

      {/* Mobile Sidebar */}
      <aside
        className={`fixed inset-y-0 left-0 w-64 bg-card border-r border-border flex flex-col z-50 transform transition-transform duration-200 ease-out md:hidden ${
          mobileMenuOpen ? 'translate-x-0' : '-translate-x-full'
        }`}
      >
        <div className="h-14 flex items-center justify-between px-4 border-b border-border">
          <div className="flex items-center gap-2.5">
            <img src="/miora.png" alt="Miora" className="w-7 h-7 rounded-lg shadow-sm" />
            <span className="font-bold text-base tracking-tight">Miora</span>
          </div>
          <button
            type="button"
            onClick={() => setMobileMenuOpen(false)}
            className="p-1.5 rounded-lg hover:bg-muted transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>
        <NavContent {...sharedNavProps} collapsed={false} />
      </aside>

      {/* Main Content */}
      <div className="flex-1 flex flex-col min-w-0 max-w-full overflow-x-hidden">
        <header className="h-14 border-b border-border flex items-center justify-between px-3 sm:px-5 md:px-8 bg-card/95 flex-shrink-0">
          <div className="flex items-center gap-2 min-w-0">
            {/* Mobile Menu Button */}
            <button
              type="button"
              onClick={() => setMobileMenuOpen(true)}
              className="p-2 rounded-lg hover:bg-muted transition-colors md:hidden mr-0.5"
              aria-label="打开导航菜单"
            >
              <Menu className="w-5 h-5" />
            </button>

            {currentGroup && (
              <>
                <span className="hidden sm:inline text-xs font-medium text-muted-foreground">
                  {currentGroup.groupLabel}
                </span>
                <span className="hidden sm:inline text-muted-foreground/45 text-xs">/</span>
              </>
            )}
            <h2 className="text-sm md:text-base font-semibold text-foreground truncate">
              {currentLabel}
            </h2>
          </div>

          {/* Quick Appearance Pill in Header */}
          <button
            type="button"
            onClick={() => setCustomizerOpen(true)}
            className="inline-flex items-center gap-2 px-2.5 py-1.5 rounded-lg border border-border bg-background/80 hover:bg-muted text-xs font-medium text-foreground transition-colors"
            title="打开外观与主题工作台"
          >
            <span
              className="w-2.5 h-2.5 rounded-full border border-black/15 dark:border-white/25 flex-shrink-0"
              style={{ backgroundColor: activePaletteMeta.swatches[0] }}
            />
            <span className="hidden sm:inline text-muted-foreground">
              {activeStyleMeta.name} ·
            </span>
            <span>{activePaletteMeta.name}</span>
            <Palette className="w-3.5 h-3.5 text-muted-foreground" />
          </button>
        </header>

        <main
          id="app-main-scroll"
          className={`flex-1 overflow-x-hidden bg-background ${
            isViewportPage ? 'overflow-hidden' : 'overflow-y-auto'
          } ${isPlayground ? 'p-2 sm:p-3 md:p-4' : 'p-2.5 sm:p-4 md:p-7'}`}
        >
          <div
            className={`${
              isPlayground ? 'max-w-[1600px]' : 'max-w-7xl'
            } mx-auto ${isViewportPage ? 'h-full' : 'min-h-full'} min-w-0 w-full`}
          >
            <Outlet />
          </div>
        </main>
      </div>

      {/* Appearance & Theme Studio */}
      <ThemeCustomizer open={customizerOpen} onOpenChange={setCustomizerOpen} />

      {/* Update Dialog */}
      <Dialog.Root open={showUpdateDialog} onOpenChange={setShowUpdateDialog}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 bg-black/50 z-[60]" />
          <Dialog.Content className="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[480px] max-w-[95vw] bg-background border border-border rounded-xl shadow-2xl z-[61] p-6">
            <div className="flex items-center justify-between gap-2 mb-4">
              <Dialog.Title className="text-lg font-bold text-foreground flex items-center gap-2">
                <ArrowUpCircle className="w-5 h-5 text-primary" /> 发现新版本
              </Dialog.Title>
              <a
                href={`https://github.com/${updateInfo?.repo || 'Mitasul/miora'}`}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md bg-muted text-xs font-mono text-muted-foreground hover:text-foreground transition-colors"
                title="版本检测上游源仓库"
              >
                <Github className="w-3.5 h-3.5" />
                {updateInfo?.repo || 'Mitasul/miora'}
              </a>
            </div>

            {updateInfo && (
              <div className="space-y-3 mb-6">
                <div className="flex items-center justify-between bg-muted p-3 rounded-lg border border-border">
                  <div>
                    <span className="text-sm text-muted-foreground">当前版本</span>
                    <div className="font-mono text-foreground">
                      v{updateInfo.current_version || currentVersion}
                    </div>
                    <span className="text-xs text-muted-foreground">{updateInfo.deploy_type}</span>
                  </div>
                  <div className="text-muted-foreground">→</div>
                  <div className="text-right">
                    <span className="text-sm text-muted-foreground">最新版本</span>
                    <div className="font-mono text-primary font-bold">
                      {updateInfo.latest_release?.tag ||
                        updateInfo.latest_pypi?.version ||
                        updateInfo.latest_docker?.tag ||
                        '—'}
                    </div>
                  </div>
                </div>

                {updateInfo.latest_release?.body && (
                  <div className="text-sm text-muted-foreground bg-muted/50 p-3 rounded-lg border border-border max-h-40 overflow-y-auto whitespace-pre-wrap">
                    {updateInfo.latest_release.body}
                  </div>
                )}

                {updateInfo.pending_count > 0 && (
                  <div className="text-sm">
                    <span className="text-muted-foreground">待拉取 </span>
                    <span className="text-foreground font-medium">
                      {updateInfo.pending_count} 个 commit
                    </span>
                    <div className="mt-1.5 bg-muted p-2 rounded-lg border border-border max-h-28 overflow-y-auto font-mono text-xs text-muted-foreground">
                      {updateInfo.pending_commits?.map((c: string, i: number) => (
                        <div key={i}>{c}</div>
                      ))}
                    </div>
                  </div>
                )}

                {updateInfo.update_instructions?.type === 'docker' &&
                  !updateInfo.update_instructions.can_auto_update && (
                    <div className="text-sm">
                      <span className="text-amber-500 font-medium">
                        ⚠ 容器未挂载 docker.sock，需手动更新：
                      </span>
                      <div className="mt-1.5 bg-muted p-2 rounded-lg border border-border font-mono text-xs text-foreground">
                        {updateInfo.update_instructions.commands?.map((c: string, i: number) => (
                          <div key={i}>$ {c}</div>
                        ))}
                      </div>
                    </div>
                  )}

                {(updateInfo.latest_release?.html_url || updateInfo.latest_pypi?.html_url) && (
                  <a
                    href={updateInfo.latest_release?.html_url || updateInfo.latest_pypi?.html_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
                  >
                    <ExternalLink className="w-3 h-3" />
                    {updateInfo.deploy_type === 'pip' ? '在 PyPI 查看' : '在 GitHub 查看'}
                  </a>
                )}
              </div>
            )}

            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleSkipVersion}
                className="px-3 py-2 text-xs text-muted-foreground hover:text-foreground transition-colors"
              >
                跳过此版本
              </button>
              <button
                type="button"
                onClick={handleSnooze}
                className="px-3 py-2 text-xs text-muted-foreground hover:text-foreground transition-colors"
              >
                24h 后提醒
              </button>
              <div className="flex-1" />
              <Dialog.Close className="px-4 py-2 text-sm font-medium text-foreground bg-muted hover:bg-muted/80 rounded-lg">
                取消
              </Dialog.Close>
              {updateInfo?.update_instructions?.type === 'docker' &&
              !updateInfo.update_instructions.can_auto_update ? (
                <button
                  type="button"
                  onClick={() => {
                    const cmds = updateInfo.update_instructions?.commands?.join('\n') || '';
                    navigator.clipboard
                      .writeText(cmds)
                      .then(() => toastSuccess('命令已复制到剪贴板'));
                  }}
                  className="px-4 py-2 text-sm font-medium bg-primary text-primary-foreground rounded-lg flex items-center gap-2"
                >
                  <Copy className="w-4 h-4" /> 复制更新命令
                </button>
              ) : (
                <button
                  type="button"
                  onClick={handlePerformUpdate}
                  disabled={updating}
                  className="px-4 py-2 text-sm font-medium bg-primary text-primary-foreground rounded-lg flex items-center gap-2 disabled:opacity-50"
                >
                  {updating ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <Download className="w-4 h-4" />
                  )}
                  {updating ? '更新中...' : '立即更新'}
                </button>
              )}
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}
