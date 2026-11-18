import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';

export type Theme = 'light' | 'dark' | 'system';
export type ThemeStyle = 'modern' | 'brutalist' | 'glass' | 'minimal';
export type ThemePalette =
  | 'sapphire'
  | 'lavender'
  | 'lemon'
  | 'muscat'
  | 'peach'
  | 'mint'
  | 'graphite';

export interface StylePresetMeta {
  id: ThemeStyle;
  name: string;
  badge: string;
  description: string;
}

export interface PalettePresetMeta {
  id: ThemePalette;
  name: string;
  subtitle: string;
  /** Preview hex swatches: [primary, softTint, darkSurface] */
  swatches: [string, string, string];
}

export const STYLE_PRESETS: StylePresetMeta[] = [
  {
    id: 'modern',
    name: '现代精致',
    badge: 'Modern',
    description: '柔和圆角、顶光微高光与自然分层阴影，兼顾质感与耐看度',
  },
  {
    id: 'brutalist',
    name: '新粗野主义',
    badge: 'Neo-Brutal',
    description: '硬朗几何折角、鲜明墨线描边与零模糊实体偏移投影，张力拉满',
  },
  {
    id: 'glass',
    name: '流光毛玻璃',
    badge: 'Frosted',
    description: '随主题流转的极光微晕底色、半透明磨砂卡片与晶莹顶边高光',
  },
  {
    id: 'minimal',
    name: '极简工业',
    badge: 'Precision',
    description: '克制小圆角、零弥散阴影与发丝级分割线，专为高密度盯盘打造',
  },
];

export const PALETTE_PRESETS: PalettePresetMeta[] = [
  {
    id: 'sapphire',
    name: '深海星蓝',
    subtitle: '经典冷静科技蓝',
    swatches: ['#2563eb', '#dbeafe', '#0f172a'],
  },
  {
    id: 'lavender',
    name: '雾霭薰衣草',
    subtitle: '优雅静谧蓝紫调',
    swatches: ['#8b5cf6', '#f3e8ff', '#1a1229'],
  },
  {
    id: 'lemon',
    name: '夏日柠檬',
    subtitle: '明快清爽冰柠黄',
    swatches: ['#eab308', '#fef9c3', '#1f190c'],
  },
  {
    id: 'muscat',
    name: '晴王青提',
    subtitle: '清透鲜活青提绿',
    swatches: ['#65a30d', '#ecfccb', '#111c13'],
  },
  {
    id: 'peach',
    name: '白桃乌龙',
    subtitle: '清甜温润蜜桃粉',
    swatches: ['#e11d48', '#ffe4e6', '#211115'],
  },
  {
    id: 'mint',
    name: '海盐薄荷',
    subtitle: '通透清冽冰川青',
    swatches: ['#0d9488', '#ccfbf1', '#0d1d1e'],
  },
  {
    id: 'graphite',
    name: '极客水墨',
    subtitle: '纯粹高对比黑白灰',
    swatches: ['#18181b', '#e4e4e7', '#09090b'],
  },
];

interface ThemeState {
  theme: Theme;
  style: ThemeStyle;
  palette: ThemePalette;
  sidebarCollapsed: boolean;
  setTheme: (theme: Theme) => void;
  setStyle: (style: ThemeStyle) => void;
  setPalette: (palette: ThemePalette) => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  toggleSidebarCollapsed: () => void;
}

const VALID_STYLES = new Set<ThemeStyle>(['modern', 'brutalist', 'glass', 'minimal']);
const VALID_PALETTES = new Set<ThemePalette>([
  'sapphire',
  'lavender',
  'lemon',
  'muscat',
  'peach',
  'mint',
  'graphite',
]);

function applyAppearance(state: Pick<ThemeState, 'theme' | 'style' | 'palette'>) {
  if (typeof window === 'undefined') return;
  const root = window.document.documentElement;

  let effectiveTheme = state.theme;
  if (state.theme === 'system') {
    effectiveTheme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }

  root.classList.toggle('dark', effectiveTheme === 'dark');

  const safeStyle: ThemeStyle = VALID_STYLES.has(state.style) ? state.style : 'modern';
  const safePalette: ThemePalette = VALID_PALETTES.has(state.palette) ? state.palette : 'sapphire';

  root.dataset.style = safeStyle;
  root.dataset.palette = safePalette;
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set, get) => ({
      theme: 'dark',
      style: 'modern',
      palette: 'sapphire',
      sidebarCollapsed: false,
      setTheme: (theme) => {
        set({ theme });
        applyAppearance({ ...get(), theme });
      },
      setStyle: (style) => {
        set({ style });
        applyAppearance({ ...get(), style });
      },
      setPalette: (palette) => {
        set({ palette });
        applyAppearance({ ...get(), palette });
      },
      setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
      toggleSidebarCollapsed: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
    }),
    {
      name: 'zoaholic-theme',
      storage: createJSONStorage(() => localStorage),
      onRehydrateStorage: () => (state) => {
        if (state) {
          applyAppearance(state);
        }
      },
    }
  )
);

useThemeStore.subscribe((state) => {
  applyAppearance(state);
});

applyAppearance(useThemeStore.getState());

if (typeof window !== 'undefined') {
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    const store = useThemeStore.getState();
    if (store.theme === 'system') {
      applyAppearance(store);
    }
  });
}
