/**
 * Refactoring UI Constraint Brief (Workflow B):
 * - Content Ranking:
 *   - Primary: Active style & palette selection cards with live visual micro-previews
 *   - Secondary: Light/System/Dark mode segmented control & live token preview strip
 *   - Tertiary: Helper descriptions, English style badges, and "恢复默认" reset link
 * - Action Hierarchy:
 *   - Primary focus: Immediate live selection of Style / Palette (no Save button needed)
 *   - Secondary: Mode switcher & Close button
 *   - Tertiary: Reset to default appearance
 * - Tokens Used:
 *   - Surfaces: bg-background, bg-card, bg-muted, border-border
 *   - Text tiers (max 3): text-foreground (600), text-foreground/85 (500), text-muted-foreground (400)
 *   - Spacing scale: p-5, space-y-6 (outer group > inner gap-2.5 per §3.6)
 */
import * as Dialog from '@radix-ui/react-dialog';
import { Check, Laptop, Moon, Palette, RotateCcw, Sparkles, Sun, X } from 'lucide-react';
import {
  PALETTE_PRESETS,
  STYLE_PRESETS,
  Theme,
  ThemeStyle,
  useThemeStore,
} from '../store/themeStore';

interface ThemeCustomizerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

function StyleWireframePreview({ styleId }: { styleId: ThemeStyle }) {
  if (styleId === 'brutalist') {
    return (
      <div className="h-12 w-full bg-muted/60 border border-foreground/75 p-2 flex items-center justify-between rounded-[2px]">
        <div className="space-y-1">
          <div className="h-2 w-12 bg-foreground/85 rounded-[1px]" />
          <div className="h-1.5 w-8 bg-muted-foreground/60 rounded-[1px]" />
        </div>
        <div
          className="h-6 px-2 bg-primary text-primary-foreground text-[10px] font-bold flex items-center border border-foreground rounded-[2px]"
          style={{ boxShadow: '2px 2px 0 0 hsl(var(--foreground))' }}
        >
          RAW
        </div>
      </div>
    );
  }

  if (styleId === 'glass') {
    return (
      <div
        className="h-12 w-full rounded-xl p-2 flex items-center justify-between relative overflow-hidden border border-border/70"
        style={{
          backgroundImage:
            'radial-gradient(circle at 18% 20%, hsl(var(--primary) / 0.28), transparent 65%), radial-gradient(circle at 85% 85%, hsl(var(--aurora-secondary) / 0.26), transparent 65%)',
        }}
      >
        <div className="bg-card/90 border border-white/40 dark:border-white/15 rounded-lg px-2.5 py-1.5 shadow-sm flex items-center gap-2 w-full justify-between">
          <div className="space-y-1">
            <div className="h-2 w-11 bg-foreground/80 rounded-full" />
            <div className="h-1.5 w-7 bg-muted-foreground/50 rounded-full" />
          </div>
          <div className="h-5 w-5 rounded-full bg-primary/90 shadow-sm" />
        </div>
      </div>
    );
  }

  if (styleId === 'minimal') {
    return (
      <div className="h-12 w-full bg-background border border-border rounded-[5px] p-2 flex items-center justify-between">
        <div className="space-y-1.5">
          <div className="h-1.5 w-14 bg-foreground/80 rounded-[1px]" />
          <div className="h-1.5 w-9 bg-muted-foreground/45 rounded-[1px]" />
        </div>
        <div className="h-5 px-2 border border-border text-[10px] font-mono text-muted-foreground flex items-center rounded-[3px]">
          1px
        </div>
      </div>
    );
  }

  // modern
  return (
    <div className="h-12 w-full bg-muted/45 rounded-xl p-1.5 flex items-center">
      <div className="w-full h-full bg-card border border-border rounded-lg px-2.5 flex items-center justify-between shadow-sm">
        <div className="space-y-1">
          <div className="h-2 w-12 bg-foreground/80 rounded-full" />
          <div className="h-1.5 w-8 bg-muted-foreground/45 rounded-full" />
        </div>
        <div className="h-5 px-2 rounded-md bg-primary text-primary-foreground text-[10px] font-semibold flex items-center shadow-sm">
          UI
        </div>
      </div>
    </div>
  );
}

export default function ThemeCustomizer({ open, onOpenChange }: ThemeCustomizerProps) {
  const { theme, style, palette, setTheme, setStyle, setPalette } = useThemeStore();

  const handleReset = () => {
    setTheme('dark');
    setStyle('modern');
    setPalette('sapphire');
  };

  const modes: { id: Theme; label: string; icon: typeof Sun }[] = [
    { id: 'light', label: '浅色', icon: Sun },
    { id: 'system', label: '跟随系统', icon: Laptop },
    { id: 'dark', label: '深色', icon: Moon },
  ];

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-black/25 z-[60]" />
        <Dialog.Content
          aria-describedby={undefined}
          style={{ contain: 'layout paint' }}
          className="fixed right-0 top-0 h-full w-full sm:w-[400px] bg-background border-l border-border shadow-2xl z-[61] flex flex-col will-change-transform animate-in slide-in-from-right duration-150 focus:outline-none"
        >
          {/* Header */}
          <div className="px-5 py-4 border-b border-border flex items-center justify-between bg-card/60">
            <Dialog.Title className="text-base font-semibold text-foreground flex items-center gap-2">
              <Palette className="w-4 h-4 text-primary" />
              外观与主题工作台
            </Dialog.Title>
            <Dialog.Close
              className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
              aria-label="关闭外观面板"
            >
              <X className="w-4 h-4" />
            </Dialog.Close>
          </div>

          {/* Scrollable Body — outer spacing space-y-6 > inner gap-2.5 (§3.6) */}
          <div className="flex-1 overflow-y-auto overscroll-contain p-5 space-y-6">
            {/* 1. 明暗模式 */}
            <section className="space-y-2.5">
              <div className="flex items-center justify-between">
                <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                  明暗模式
                </h3>
              </div>
              <div className="grid grid-cols-3 gap-1.5 p-1 bg-muted/75 rounded-lg border border-border/60">
                {modes.map((m) => {
                  const Icon = m.icon;
                  const active = theme === m.id;
                  return (
                    <button
                      key={m.id}
                      type="button"
                      onClick={() => setTheme(m.id)}
                      className={`flex items-center justify-center gap-1.5 py-2 px-2 rounded-md text-xs font-medium transition-colors ${
                        active
                          ? 'bg-card text-foreground shadow-sm border border-border/80 font-semibold'
                          : 'text-muted-foreground hover:text-foreground'
                      }`}
                    >
                      <Icon className="w-3.5 h-3.5" />
                      {m.label}
                    </button>
                  );
                })}
              </div>
            </section>

            {/* 2. 视觉风格 */}
            <section className="space-y-2.5">
              <div className="flex items-center justify-between">
                <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                  设计风格
                </h3>
                <span className="text-[11px] text-muted-foreground">全局圆角 · 阴影 · 材质</span>
              </div>
              <div className="grid grid-cols-2 gap-2.5">
                {STYLE_PRESETS.map((preset) => {
                  const active = style === preset.id;
                  return (
                    <button
                      key={preset.id}
                      type="button"
                      onClick={() => setStyle(preset.id)}
                      className={`text-left p-2.5 rounded-xl border transition-colors flex flex-col gap-2 relative ${
                        active
                          ? 'border-primary bg-primary/[0.05] ring-1 ring-primary'
                          : 'border-border bg-card hover:border-foreground/30 hover:bg-muted/30'
                      }`}
                    >
                      <StyleWireframePreview styleId={preset.id} />
                      <div className="space-y-0.5">
                        <div className="flex items-center justify-between gap-1">
                          <span className="text-xs font-semibold text-foreground">
                            {preset.name}
                          </span>
                          {active ? (
                            <span className="w-4 h-4 rounded-full bg-primary text-primary-foreground flex items-center justify-center flex-shrink-0">
                              <Check className="w-2.5 h-2.5 stroke-[3]" />
                            </span>
                          ) : (
                            <span className="text-[10px] font-mono text-muted-foreground">
                              {preset.badge}
                            </span>
                          )}
                        </div>
                        <p className="text-[11px] leading-snug text-muted-foreground line-clamp-2">
                          {preset.description}
                        </p>
                      </div>
                    </button>
                  );
                })}
              </div>
            </section>

            {/* 3. 精选配色 */}
            <section className="space-y-2.5">
              <div className="flex items-center justify-between">
                <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                  精选配色
                </h3>
                <span className="text-[11px] text-muted-foreground">含同色温背景染色</span>
              </div>
              <div className="grid grid-cols-1 gap-2">
                {PALETTE_PRESETS.map((p) => {
                  const active = palette === p.id;
                  return (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => setPalette(p.id)}
                      className={`w-full flex items-center gap-3 p-2.5 rounded-xl border text-left transition-colors ${
                        active
                          ? 'border-primary bg-primary/[0.06] ring-1 ring-primary'
                          : 'border-border bg-card hover:border-foreground/30 hover:bg-muted/30'
                      }`}
                    >
                      {/* Layered color trio */}
                      <div className="flex items-center -space-x-1.5 flex-shrink-0 pl-0.5">
                        <span
                          className="w-5 h-5 rounded-full border border-black/15 dark:border-white/20 shadow-sm z-30"
                          style={{ backgroundColor: p.swatches[0] }}
                        />
                        <span
                          className="w-5 h-5 rounded-full border border-black/10 dark:border-white/15 z-20"
                          style={{ backgroundColor: p.swatches[1] }}
                        />
                        <span
                          className="w-5 h-5 rounded-full border border-black/15 dark:border-white/20 z-10"
                          style={{ backgroundColor: p.swatches[2] }}
                        />
                      </div>

                      <div className="flex-1 min-w-0">
                        <div className="text-sm font-semibold text-foreground leading-tight">
                          {p.name}
                        </div>
                        <div className="text-xs text-muted-foreground truncate mt-0.5">
                          {p.subtitle}
                        </div>
                      </div>

                      {active && (
                        <span className="w-5 h-5 rounded-full bg-primary text-primary-foreground flex items-center justify-center flex-shrink-0">
                          <Check className="w-3 h-3 stroke-[2.75]" />
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            </section>

            {/* 4. 实时质感微缩预览 */}
            <section className="space-y-2">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                当前层次与对比度预览
              </h3>
              <div className="bg-card border border-border rounded-xl p-3.5 space-y-3">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-xs text-muted-foreground">实时吞吐 · Tabular</div>
                    <div className="text-lg font-bold font-mono tabular-nums text-foreground">
                      192.4 <span className="text-xs font-normal text-muted-foreground">tok/s</span>
                    </div>
                  </div>
                  <span className="inline-flex items-center gap-1 text-xs font-medium px-2 py-0.5 rounded-md bg-primary/15 text-primary">
                    <Sparkles className="w-3 h-3" /> WCAG AA
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    className="bg-primary text-primary-foreground text-xs font-semibold px-3 py-1.5 rounded-lg"
                  >
                    主操作按钮
                  </button>
                  <button
                    type="button"
                    className="bg-secondary text-secondary-foreground border border-border text-xs font-medium px-3 py-1.5 rounded-lg"
                  >
                    次级按钮
                  </button>
                </div>
              </div>
            </section>
          </div>

          {/* Footer */}
          <div className="px-5 py-3.5 border-t border-border bg-card/60 flex items-center justify-between">
            <button
              type="button"
              onClick={handleReset}
              className="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors"
            >
              <RotateCcw className="w-3.5 h-3.5" />
              恢复默认外观
            </button>
            <Dialog.Close className="px-3.5 py-1.5 text-xs font-semibold bg-primary text-primary-foreground rounded-lg hover:opacity-95 transition-opacity">
              完成
            </Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
