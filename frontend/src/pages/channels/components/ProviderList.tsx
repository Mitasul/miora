import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { ArrowDown, ArrowUp, ArrowUpDown, BarChart3, Boxes, CheckCircle2, Copy, Edit, Files, Folder, MoreHorizontal, Play, Power, Puzzle, Search, Trash2, X, XCircle } from 'lucide-react';
import type React from 'react';
import { Fragment, useEffect, useMemo, useRef, useState } from 'react';
import { ProviderBatchActions } from './ProviderBatchActions';
import { selectedProviderNames, type BatchProvider } from '../lib/batchProviders';
import { toastSuccess } from '../../../components/Toast';

import { ProviderLogo } from '../../../components/ProviderLogos';
import type { Segment } from '../types';

// 修改原因：渠道列表渲染占据 ChannelsPage 大部分 JSX，需要独立成组件。
// 修改方式：把原筛选栏、移动端卡片、桌面表格和不活跃分组整体迁移，并用 props 注入 handlers。
// 目的：保持列表交互不变，同时让页面骨架更清晰。

const batchButtonClass = 'px-3 py-2 text-sm rounded-lg border border-border hover:bg-muted transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:opacity-50 disabled:pointer-events-none';
const batchCheckboxClass = 'h-4 w-4 shrink-0 rounded border-border accent-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:opacity-50';

function extractModelLabels(provider: any): string[] {
  const raw = Array.isArray(provider?.model)
    ? provider.model
    : Array.isArray(provider?.models)
    ? provider.models
    : [];
  const names: string[] = [];
  for (const item of raw) {
    if (typeof item === 'string' && item.trim()) {
      names.push(item.trim());
    } else if (item && typeof item === 'object') {
      for (const [upstream, alias] of Object.entries(item)) {
        const label = typeof alias === 'string' && alias.trim() ? alias.trim() : upstream.trim();
        if (label) names.push(label);
      }
    }
  }
  return names;
}

type SortField = 'default' | 'name' | 'keys' | 'models' | 'status' | 'weight';
type SortDir = 'desc' | 'asc';

function extractShortHost(baseUrl?: string): string {
  if (!baseUrl || typeof baseUrl !== 'string') return '';
  try {
    const u = new URL(baseUrl.trim());
    return u.host;
  } catch {
    return baseUrl.replace(/^https?:\/\//, '').split('/')[0] || '';
  }
}

export interface ProviderListProps {
  providers: BatchProvider[];
  token: string | null;
  refreshProviders: () => Promise<void>;
  loading: boolean;
  totalListItemCount: number;
  visibleListItemCount: number;
  filterKeyword: string;
  setFilterKeyword: (value: string) => void;
  filterEngine: string;
  setFilterEngine: (value: string) => void;
  filterGroup: string;
  setFilterGroup: (value: string) => void;
  filterStatus: '' | 'enabled' | 'disabled';
  setFilterStatus: (value: '' | 'enabled' | 'disabled') => void;
  availableEngines: string[];
  availableGroups: string[];
  hasActiveFilters: string | boolean;
  segments: Segment[];
  expandedInactiveGroups: Set<number>;
  toggleInactiveGroup: (groupKey: number) => void;
  runtimeKeyStatus: Record<string, { auto_disabled: { key: string; remaining_seconds: number; duration: number; reason: string }[]; cooling: any[] }>;
  getMatchedModels: (provider: any) => string[];
  getProviderAnalyticsName: (provider: any) => string;
  setAnalyticsProvider: (providerName: string) => void;
  setAnalyticsOpen: (open: boolean) => void;
  openTestDialog: (provider: any) => void;
  handleToggleProvider: (idx: number) => void;
  handleCopyProvider: (provider: any) => void;
  openModal: (provider?: any, index?: number | null) => void;
  handleDeleteProvider: (idx: number) => void;
  handleUpdateWeight: (idx: number, newWeight: number) => void;
  buildSubChannelProvider: (parentIdx: number, subIdx: number) => any | null;
  handleToggleSubChannel: (parentIdx: number, subIdx: number) => void;
  openSubChannelEdit: (parentIdx: number, subIdx: number) => void;
  handleDeleteSubChannel: (parentIdx: number, subIdx: number) => void;
  renderMobileVirtualRoutesAccordion: () => React.ReactNode;
  renderDesktopVirtualRoutesAccordionRows: () => React.ReactNode;
}

export function ProviderList({
  providers, token, refreshProviders,
  loading, totalListItemCount, visibleListItemCount, filterKeyword, setFilterKeyword, filterEngine, setFilterEngine,
  filterGroup, setFilterGroup, filterStatus, setFilterStatus, availableEngines, availableGroups, hasActiveFilters,
  segments, expandedInactiveGroups, toggleInactiveGroup, runtimeKeyStatus, getMatchedModels, getProviderAnalyticsName,
  setAnalyticsProvider, setAnalyticsOpen, openTestDialog, handleToggleProvider, handleCopyProvider, openModal, handleDeleteProvider,
  handleUpdateWeight, buildSubChannelProvider, handleToggleSubChannel, openSubChannelEdit, handleDeleteSubChannel,
  renderMobileVirtualRoutesAccordion, renderDesktopVirtualRoutesAccordionRows,
}: ProviderListProps) {
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  const [sortField, setSortField] = useState<SortField>('default');
  const [sortDir, setSortDir] = useState<SortDir>('desc');

  const toggleSort = (field: Exclude<SortField, 'default'>) => {
    if (sortField !== field) {
      setSortField(field);
      setSortDir(field === 'name' ? 'asc' : 'desc');
    } else if (sortDir === (field === 'name' ? 'asc' : 'desc')) {
      setSortDir(field === 'name' ? 'desc' : 'asc');
    } else {
      setSortField('default');
    }
  };

  const renderSortIcon = (field: Exclude<SortField, 'default'>) => {
    if (sortField !== field) return <ArrowUpDown className="w-3 h-3 opacity-45 group-hover/th:opacity-85 transition-opacity" />;
    return sortDir === 'asc'
      ? <ArrowUp className="w-3 h-3 text-primary" />
      : <ArrowDown className="w-3 h-3 text-primary" />;
  };
  const visibleNames = useMemo(() => segments.flatMap(segment =>
    segment.type === 'active' ? [segment.item.p.provider] : segment.items.map(item => item.p.provider)), [segments]);
  const selectedNames = selectedProviderNames(visibleNames, selected);
  useEffect(() => { setSelected(new Set()); }, [filterKeyword, filterEngine, filterGroup, filterStatus]);
  const toggleSelected = (name: string) => {
    if (batchBusy) return;
    setSelected(previous => {
      const next = new Set(previous);
      if (next.has(name)) next.delete(name); else next.add(name);
      return next;
    });
  };
  // 修改原因：ProviderCard 原定义在组件函数体内，每次渲染都产生新组件类型，React 会把全部移动端卡片卸载重建，滚动锚定失效后页面回弹。
  // 修改方式：用 useMemo 固定组件类型，处理器通过 ref 透传保持最新。
  // 目的：列表重渲染只更新属性而不再重建 DOM 节点，移动端滚动位置保持稳定。
  const cardActionsRef = useRef({} as any);
  cardActionsRef.current = {
    getProviderAnalyticsName, setAnalyticsProvider, setAnalyticsOpen, openTestDialog,
    handleToggleProvider, handleCopyProvider, openModal, handleDeleteProvider,
    handleUpdateWeight, buildSubChannelProvider, handleToggleSubChannel,
    openSubChannelEdit, handleDeleteSubChannel,
    selected, toggleSelected, batchBusy, selecting,
  };
  const ProviderCard = useMemo(() => ({ p, idx }: { p: any; idx: number }) => {
    const {
      getProviderAnalyticsName, setAnalyticsProvider, setAnalyticsOpen, openTestDialog,
      handleToggleProvider, handleCopyProvider, openModal, handleDeleteProvider,
      handleUpdateWeight, buildSubChannelProvider, handleToggleSubChannel,
      openSubChannelEdit, handleDeleteSubChannel,
      selected, toggleSelected, batchBusy, selecting,
    } = cardActionsRef.current;
    const isEnabled = p.enabled !== false;
    const groups = Array.isArray(p.groups) ? p.groups : p.group ? [p.group] : ['default'];
    const plugins = p.preferences?.enabled_plugins || [];
    const modelLabels = extractModelLabels(p);
    const weight = p.preferences?.weight ?? p.weight ?? 0;

    return (
      <div className={`bg-card border border-border rounded-xl p-4 ${!isEnabled && 'opacity-60'}`}>
        <div className="flex items-start justify-between mb-3">
          <div className="flex items-center gap-3">
            {selecting && <input type="checkbox" className={batchCheckboxClass} aria-label={`选择渠道 ${p.provider}`} checked={selected.has(p.provider)} disabled={batchBusy} onChange={() => toggleSelected(p.provider)} />}
            <ProviderLogo name={p.provider} engine={p.engine} baseUrl={p.base_url} />
            <div>
              <div className={`font-semibold ${isEnabled ? 'text-foreground' : 'text-muted-foreground'}`}>{p.provider}</div>
              <div className="text-xs text-muted-foreground font-mono">{p.engine || 'openai'}</div>
              {p.remark && (
                <div className="mt-1 text-xs text-muted-foreground break-words whitespace-pre-wrap max-w-full">
                  {p.remark}
                </div>
              )}
            </div>
          </div>
          <button
            type="button"
            onClick={() => handleToggleProvider(idx)}
            className={`inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium transition-colors ${isEnabled ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 hover:bg-emerald-500/20' : 'bg-muted text-muted-foreground hover:bg-muted/80'}`}
          >
            {isEnabled ? <CheckCircle2 className="w-3 h-3" /> : <X className="w-3 h-3" />}
            {isEnabled ? '启用' : '禁用'}
          </button>
        </div>

        <div className="flex flex-wrap gap-1.5 mb-3">
          {groups.map((g: string, i: number) => (
            <span key={i} className="flex items-center gap-1 bg-muted text-foreground px-2 py-0.5 rounded text-xs"><Folder className="w-3 h-3 text-muted-foreground" />{g}</span>
          ))}
          {modelLabels.length > 0 && (
            <span
              className="bg-secondary text-secondary-foreground px-2 py-0.5 rounded text-xs font-mono tabular-nums flex items-center gap-1"
              title={modelLabels.slice(0, 15).join(', ') + (modelLabels.length > 15 ? ` …共 ${modelLabels.length} 个` : '')}
            >
              <Boxes className="w-3 h-3 text-muted-foreground" /> {modelLabels.length} 模型
            </span>
          )}
          {plugins.length > 0 && (
            <span className="bg-primary/10 text-primary px-2 py-0.5 rounded text-xs font-mono tabular-nums flex items-center gap-1" title={plugins.join(', ')}>
              <Puzzle className="w-3 h-3" /> {plugins.length}
            </span>
          )}
        </div>

        <div className="flex items-center justify-between pt-3 border-t border-border gap-2">
          <div className="flex items-center gap-1.5 flex-shrink-0">
            <span className="text-xs text-muted-foreground">权重</span>
            <input
              type="number"
              value={weight}
              onChange={e => handleUpdateWeight(idx, parseInt(e.target.value) || 0)}
              className="w-12 bg-muted border border-border rounded px-1.5 py-1 text-center font-mono tabular-nums text-xs text-foreground"
            />
          </div>
          <div className="flex items-center gap-1 flex-shrink-0">
            <button onClick={() => openTestDialog(p)} className="px-2 py-1 text-xs font-medium text-foreground bg-muted/70 hover:bg-primary/10 hover:text-primary rounded-md transition-colors inline-flex items-center gap-1" title="测试">
              <Play className="w-3.5 h-3.5" /> 测试
            </button>
            <button onClick={() => { setAnalyticsProvider(getProviderAnalyticsName(p)); setAnalyticsOpen(true); }} className="p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors" title="用量分析">
              <BarChart3 className="w-4 h-4" />
            </button>
            <button onClick={() => openModal(p, idx)} className="p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors" title="编辑">
              <Edit className="w-4 h-4" />
            </button>
            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <button type="button" className="p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors" aria-label="更多操作">
                  <MoreHorizontal className="w-4 h-4" />
                </button>
              </DropdownMenu.Trigger>
              <DropdownMenu.Portal>
                <DropdownMenu.Content align="end" sideOffset={4} className="min-w-[140px] bg-popover text-popover-foreground border border-border rounded-lg shadow-lg p-1 z-50 text-xs">
                  <DropdownMenu.Item onSelect={() => handleToggleProvider(idx)} className="flex items-center gap-2 px-2.5 py-1.5 rounded-md cursor-pointer outline-none hover:bg-muted">
                    <Power className="w-3.5 h-3.5 text-muted-foreground" />
                    {isEnabled ? '禁用渠道' : '启用渠道'}
                  </DropdownMenu.Item>
                  <DropdownMenu.Item onSelect={() => handleCopyProvider(p)} className="flex items-center gap-2 px-2.5 py-1.5 rounded-md cursor-pointer outline-none hover:bg-muted">
                    <Files className="w-3.5 h-3.5 text-muted-foreground" />
                    复制渠道
                  </DropdownMenu.Item>
                  <DropdownMenu.Separator className="my-1 h-px bg-border" />
                  <DropdownMenu.Item onSelect={() => handleDeleteProvider(idx)} className="flex items-center gap-2 px-2.5 py-1.5 rounded-md cursor-pointer outline-none text-destructive hover:bg-destructive/10">
                    <Trash2 className="w-3.5 h-3.5" />
                    删除渠道
                  </DropdownMenu.Item>
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu.Root>
          </div>
        </div>

        {/* 子渠道列表 */}
        {(p.sub_channels || []).length > 0 && (
          <div className="mt-3 pt-3 border-t border-border space-y-2">
            <div className="text-[10px] text-muted-foreground font-medium uppercase tracking-wider">子渠道</div>
            {(p.sub_channels || []).map((sub: any, subIdx: number) => {
              const subEnabled = sub.enabled !== false;
              const subModels = Array.isArray(sub.model) ? sub.model : Array.isArray(sub.models) ? sub.models : [];
              return (
                <div key={subIdx} className={`flex items-center justify-between bg-muted/30 rounded-lg px-3 py-2 ${!subEnabled && 'opacity-50'}`}>
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="text-muted-foreground text-xs">└</span>
                    <div className="min-w-0">
                      <div className="text-xs font-medium text-foreground truncate">{sub.remark || sub.engine || '?'}</div>
                      <div className="text-[10px] text-muted-foreground">{subModels.length} 模型</div>
                    </div>
                    {!subEnabled && <span className="text-[10px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-500 flex-shrink-0">禁用</span>}
                  </div>
                  <div className="flex items-center gap-0.5 flex-shrink-0">
                    <button onClick={() => { const sp = buildSubChannelProvider(idx, subIdx); if (sp) openTestDialog(sp); }} className="p-1 text-blue-600 dark:text-blue-400 hover:bg-blue-500/10 rounded-md transition-colors" title="测试子渠道">
                      <Play className="w-3.5 h-3.5" />
                    </button>
                    <button onClick={() => handleToggleSubChannel(idx, subIdx)} className={`p-1 rounded-md transition-colors ${subEnabled ? 'text-emerald-600 dark:text-emerald-500 hover:bg-emerald-500/10' : 'text-muted-foreground hover:bg-muted'}`} title={subEnabled ? '禁用' : '启用'}>
                      <Power className="w-3.5 h-3.5" />
                    </button>
                    <button onClick={() => openSubChannelEdit(idx, subIdx)} className="p-1 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors" title="编辑子渠道">
                      <Edit className="w-3.5 h-3.5" />
                    </button>
                    <button onClick={() => handleDeleteSubChannel(idx, subIdx)} className="p-1 text-red-600 dark:text-red-500 hover:bg-red-500/10 rounded-md transition-colors" title="删除子渠道">
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    );
  }, []);


  return (
    <>
      {/* ── Filter Bar ── */}
      {!loading && totalListItemCount > 0 && (
        <div className="grid grid-cols-2 items-center gap-2 lg:flex lg:flex-wrap">
          {/* 搜索框 */}
          <div className="relative col-span-2 min-w-0 lg:flex-1 lg:min-w-48">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none" />
            <input
              type="text"
              value={filterKeyword}
              onChange={e => setFilterKeyword(e.target.value)}
              placeholder="搜索渠道名、备注、模型名…"
              className="w-full bg-background border border-border rounded-lg pl-9 pr-8 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:border-primary outline-none"
            />
            {filterKeyword && (
              <button
                onClick={() => setFilterKeyword('')}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              >
                <XCircle className="w-4 h-4" />
              </button>
            )}
          </div>

          {/* 引擎筛选 */}
          <select
            value={filterEngine}
            onChange={e => setFilterEngine(e.target.value)}
            className="bg-background border border-border rounded-lg px-3 py-2 text-sm text-foreground min-w-[120px]"
          >
            <option value="">全部引擎</option>
            {availableEngines.map(eng => (
              <option key={eng} value={eng}>{eng}</option>
            ))}
          </select>

          {/* 分组筛选 */}
          <select
            value={filterGroup}
            onChange={e => setFilterGroup(e.target.value)}
            className="bg-background border border-border rounded-lg px-3 py-2 text-sm text-foreground min-w-[120px]"
          >
            <option value="">全部分组</option>
            {availableGroups.map(g => (
              <option key={g} value={g}>{g}</option>
            ))}
          </select>

          {/* 状态筛选 */}
          <select
            value={filterStatus}
            onChange={e => setFilterStatus(e.target.value as '' | 'enabled' | 'disabled')}
            className="bg-background border border-border rounded-lg px-3 py-2 text-sm text-foreground min-w-[100px]"
          >
            <option value="">全部状态</option>
            <option value="enabled">已启用</option>
            <option value="disabled">已禁用</option>
          </select>

          <button id="provider-selection-toggle" type="button" disabled={batchBusy || !visibleNames.length} aria-pressed={selecting} onClick={() => { setSelecting(value => !value); setSelected(new Set()); }} className={`${batchButtonClass} text-muted-foreground ${selecting ? 'border-primary/40 bg-primary/5 text-primary' : ''}`}>{selecting ? '退出选择' : '选择'}</button>

          {sortField !== 'default' && (
            <button
              type="button"
              onClick={() => setSortField('default')}
              className="flex items-center gap-1 px-3 py-2 text-xs font-medium text-primary bg-primary/10 hover:bg-primary/15 rounded-lg transition-colors flex-shrink-0"
              title="恢复按调度优先级与活跃度分段顺序显示"
            >
              <X className="w-3 h-3" /> 恢复默认顺序
            </button>
          )}

          {/* 清除筛选 */}
          {hasActiveFilters && (
            <button
              onClick={() => { setFilterKeyword(''); setFilterEngine(''); setFilterGroup(''); setFilterStatus(''); }}
              className="flex items-center gap-1 px-3 py-2 text-xs text-muted-foreground hover:text-foreground bg-muted hover:bg-muted/80 rounded-lg transition-colors flex-shrink-0"
            >
              <X className="w-3 h-3" /> 清除
            </button>
          )}
        </div>
      )}

      {/* 筛选结果统计 */}
      {!loading && hasActiveFilters && (
        <div className="text-xs text-muted-foreground">
          筛选结果：{visibleListItemCount}/{totalListItemCount} 个条目
          {filterKeyword && visibleListItemCount > 0 && (
            <span className="ml-2 text-primary">含模型名或链条匹配</span>
          )}
        </div>
      )}

      {/* Keep the action component mounted while refresh/clearSelection runs so its result dialog survives. */}
      <div className={selecting ? 'fixed inset-x-0 bottom-0 z-40 flex flex-wrap items-center gap-2 border-t border-border bg-background/95 p-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] shadow-lg backdrop-blur-md md:static md:border-0 md:border-b md:bg-transparent md:px-0 md:py-2 md:shadow-none md:backdrop-blur-none' : 'hidden'}>
        <span className="mr-auto text-sm font-medium" aria-live="polite">已选 {selectedNames.length} 个</span>
        <button type="button" aria-label="全选当前筛选渠道" title="选择当前筛选结果，包含折叠渠道" disabled={batchBusy || !visibleNames.length} onClick={() => setSelected(new Set(visibleNames))} className={batchButtonClass}>全选</button>
        <button type="button" aria-label="取消渠道选择" disabled={batchBusy} onClick={() => { setSelecting(false); setSelected(new Set()); }} className={batchButtonClass}>取消</button>
        <div className="w-full md:w-auto"><ProviderBatchActions providers={providers} selectedNames={selectedNames} token={token} refresh={refreshProviders} busy={batchBusy} setBusy={setBatchBusy} clearSelection={() => setSelected(new Set())} /></div>
      </div>
      <div className={selecting ? 'pb-[calc(10rem_+_env(safe-area-inset-bottom))] md:pb-0' : ''}>
      <div id="channel-provider-list">
      {/* Mobile Card List */}
      <div className="md:hidden space-y-4">
        {loading ? (
          <div className="p-8 text-center text-muted-foreground">加载中...</div>
        ) : visibleListItemCount === 0 ? (
          <div className="p-12 text-center text-muted-foreground">{totalListItemCount === 0 ? '暂无渠道配置，点击上方按钮添加。' : '没有符合筛选条件的渠道。'}</div>
        ) : (
          <>
            {renderMobileVirtualRoutesAccordion()}
            {segments.map((seg, si) => seg.type === 'active' ? (
              <ProviderCard key={`a-${seg.item.idx}-${seg.item.p.provider || si}`} p={seg.item.p} idx={seg.item.idx} />
            ) : (
              <div key={`i-${seg.startIndex}`} className="border border-border rounded-xl overflow-hidden">
                <button onClick={() => toggleInactiveGroup(seg.startIndex)} className="w-full flex items-center justify-between px-4 py-3 bg-muted/30 hover:bg-muted/50 transition-colors text-sm">
                  <span className="text-muted-foreground">不活跃渠道 ({seg.items.length})</span>
                  <span className="text-xs text-muted-foreground">{expandedInactiveGroups.has(seg.startIndex) ? '▲' : '▼'}</span>
                </button>
                {expandedInactiveGroups.has(seg.startIndex) && (
                  <div className="space-y-4 p-2 opacity-70">
                    {seg.items.map(({ p, idx }) => <ProviderCard key={idx} p={p} idx={idx} />)}
                  </div>
                )}
              </div>
            ))}
          </>
        )}
      </div>

      {/* Desktop Table */}
      <div className="hidden md:block bg-card border border-border rounded-xl overflow-hidden">
        {loading ? (
          <div className="p-8 text-center text-muted-foreground">加载中...</div>
        ) : visibleListItemCount === 0 ? (
          <div className="p-12 text-center text-muted-foreground">{totalListItemCount === 0 ? '暂无渠道配置，点击右上角添加。' : '没有符合筛选条件的渠道。'}</div>
        ) : (
          <table className="w-full text-left border-collapse table-fixed">
            <thead className="bg-muted/60 border-b border-border text-muted-foreground text-xs uppercase tracking-wider font-semibold select-none">
              <tr>
                <th className="px-4 py-3 w-[25%]">
                  <button type="button" onClick={() => toggleSort('name')} className="group/th inline-flex items-center gap-1 hover:text-foreground transition-colors">
                    渠道名称 {renderSortIcon('name')}
                  </button>
                </th>
                <th className="px-4 py-3 w-[15%]">分组 / 引擎</th>
                <th className="px-4 py-3 w-[9%] text-center">
                  <button type="button" onClick={() => toggleSort('keys')} className="group/th inline-flex items-center justify-center gap-1 hover:text-foreground transition-colors">
                    Keys {renderSortIcon('keys')}
                  </button>
                </th>
                <th className="px-4 py-3 w-[15%]">
                  <button type="button" onClick={() => toggleSort('models')} className="group/th inline-flex items-center gap-1 hover:text-foreground transition-colors">
                    模型 / 插件 {renderSortIcon('models')}
                  </button>
                </th>
                <th className="px-4 py-3 w-[8%] text-center">
                  <button type="button" onClick={() => toggleSort('status')} className="group/th inline-flex items-center justify-center gap-1 hover:text-foreground transition-colors">
                    状态 {renderSortIcon('status')}
                  </button>
                </th>
                <th className="px-4 py-3 w-[9%] text-center">
                  <button type="button" onClick={() => toggleSort('weight')} className="group/th inline-flex items-center justify-center gap-1 hover:text-foreground transition-colors">
                    权重 {renderSortIcon('weight')}
                  </button>
                </th>
                <th className="px-4 py-3 w-[19%] text-right">操作</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border text-sm">
              {renderDesktopVirtualRoutesAccordionRows()}
              {(() => {
                const countEffectiveKeys = (prov: any) => {
                  const raw = Array.isArray(prov.api) ? prov.api : (typeof prov.api === 'string' && prov.api.trim() ? [prov.api] : []);
                  const cfgDisabled = raw.filter((k: any) => {
                    if (typeof k === 'string') return k.startsWith('!');
                    if (k && typeof k === 'object') return (Object.keys(k)[0] || '').startsWith('!');
                    return false;
                  }).length;
                  const rtDis = runtimeKeyStatus[prov.provider]?.auto_disabled?.length || 0;
                  return Math.max(0, raw.length - cfgDisabled - rtDis);
                };

                const rows: any[] = [];
                if (sortField === 'default') {
                  segments.forEach((seg) => {
                    if (seg.type === 'active') {
                      rows.push({ p: seg.item.p, idx: seg.item.idx, inactive: false });
                    } else {
                      rows.push({ type: 'collapse-btn', startIndex: seg.startIndex, count: seg.items.length });
                      if (expandedInactiveGroups.has(seg.startIndex)) {
                        seg.items.forEach(item => rows.push({ p: item.p, idx: item.idx, inactive: true }));
                      }
                    }
                  });
                } else {
                  const flatItems = segments.flatMap((seg) =>
                    seg.type === 'active'
                      ? [{ p: seg.item.p, idx: seg.item.idx, inactive: false }]
                      : seg.items.map((it) => ({ p: it.p, idx: it.idx, inactive: true }))
                  );
                  flatItems.sort((a, b) => {
                    let cmp = 0;
                    if (sortField === 'name') {
                      cmp = String(a.p.provider || '').localeCompare(String(b.p.provider || ''), 'zh-CN');
                    } else if (sortField === 'keys') {
                      cmp = countEffectiveKeys(a.p) - countEffectiveKeys(b.p);
                    } else if (sortField === 'models') {
                      cmp = extractModelLabels(a.p).length - extractModelLabels(b.p).length;
                    } else if (sortField === 'status') {
                      const sa = a.p.enabled !== false ? 1 : 0;
                      const sb = b.p.enabled !== false ? 1 : 0;
                      cmp = sa - sb;
                    } else if (sortField === 'weight') {
                      const wa = a.p.preferences?.weight ?? a.p.weight ?? 0;
                      const wb = b.p.preferences?.weight ?? b.p.weight ?? 0;
                      cmp = wa - wb;
                    }
                    return sortDir === 'asc' ? cmp : -cmp;
                  });
                  rows.push(...flatItems);
                }
                return rows.map((row) => {
                  if (row.type === 'collapse-btn') {
                    return (
                      <tr key={`ig-${row.startIndex}`}>
                        <td colSpan={7} className="p-0">
                          <button onClick={() => toggleInactiveGroup(row.startIndex)} className="w-full flex items-center justify-between px-4 py-2 bg-muted/20 hover:bg-muted/40 transition-colors">
                            <span className="text-muted-foreground text-xs">不活跃渠道 ({row.count})</span>
                            <span className="text-xs text-muted-foreground">{expandedInactiveGroups.has(row.startIndex) ? '▲' : '▼'}</span>
                          </button>
                        </td>
                      </tr>
                    );
                  }
                  const { p, idx, inactive: isInactive } = row;
                  const isEnabled = p.enabled !== false;
                  const groups = Array.isArray(p.groups) ? p.groups : p.group ? [p.group] : ['default'];
                  const plugins = p.preferences?.enabled_plugins || [];
                  const weight = p.preferences?.weight ?? p.weight ?? 0;

                  // Key 统计
                  const apiRaw = Array.isArray(p.api) ? p.api : (typeof p.api === 'string' && p.api.trim() ? [p.api] : []);
                  const totalKeys = apiRaw.length;
                  const configDisabledKeys = apiRaw.filter((k: any) => {
                    if (typeof k === 'string') return k.startsWith('!');
                    if (k && typeof k === 'object') { const key = Object.keys(k)[0] || ''; return key.startsWith('!'); }
                    return false;
                  }).length;
                  const rtStatus = runtimeKeyStatus[p.provider];
                  const rtDisabledCount = rtStatus?.auto_disabled?.length || 0;
                  const rtCoolingCount = rtStatus?.cooling?.length || 0;
                  const enabledKeys = totalKeys - configDisabledKeys;
                  const effectiveEnabled = Math.max(0, enabledKeys - rtDisabledCount);
                  const hasKeyIssue = configDisabledKeys > 0 || rtDisabledCount > 0 || rtCoolingCount > 0;
                  const modelLabels = extractModelLabels(p);
                  const shortHost = extractShortHost(p.base_url);

                  // 模型名匹配高亮
                  const matchedModels = getMatchedModels(p);

                  return (<Fragment key={p.provider}>
                  <tr key={idx} className={`group/row transition-colors ${isInactive ? 'opacity-50' : ''} ${isEnabled ? 'hover:bg-muted/50' : 'bg-muted/30 opacity-60'}`}>
                    <td className="px-4 py-3 align-top">
                      <div className="flex items-start gap-2.5">
                        {selecting && <input type="checkbox" className={`${batchCheckboxClass} mt-1`} aria-label={`选择渠道 ${p.provider}`} checked={selected.has(p.provider)} disabled={batchBusy} onChange={() => toggleSelected(p.provider)} />}
                        <div className="mt-0.5 flex-shrink-0">
                          <ProviderLogo name={p.provider} engine={p.engine} baseUrl={p.base_url} />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className={`font-medium break-words ${isEnabled ? 'text-foreground' : 'text-muted-foreground'}`}>{p.provider}</div>
                          {shortHost && (
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                navigator.clipboard.writeText(p.base_url).then(() => toastSuccess('已复制上游地址'));
                              }}
                              className="inline-flex items-center gap-1 font-mono text-[11px] text-muted-foreground/80 hover:text-foreground truncate max-w-full mt-0.5"
                              title={`点击复制: ${p.base_url}`}
                            >
                              <span className="truncate">{shortHost}</span>
                              <Copy className="w-2.5 h-2.5 flex-shrink-0 opacity-0 group-hover/row:opacity-70 transition-opacity" />
                            </button>
                          )}
                          {p.remark && (
                            <div className="text-xs text-muted-foreground whitespace-pre-wrap break-words leading-relaxed mt-1">
                              {p.remark}
                            </div>
                          )}
                          {matchedModels.length > 0 && (
                            <div className="flex flex-wrap gap-0.5 mt-1">
                              {matchedModels.slice(0, 2).map((m, i) => (
                                <span key={i} className="text-[10px] font-mono px-1 py-px rounded bg-primary/10 text-primary truncate max-w-[140px]" title={m}>{m}</span>
                              ))}
                              {matchedModels.length > 2 && <span className="text-[10px] text-muted-foreground">+{matchedModels.length - 2}</span>}
                            </div>
                          )}
                        </div>
                      </div>
                    </td>
                    <td className="px-4 py-3">
                      <div className="flex flex-col gap-1">
                        <div className="flex gap-1 flex-wrap">
                          {groups.slice(0, 2).map((g: string, i: number) => (
                            <span key={i} className="bg-muted text-foreground px-1.5 py-0.5 rounded text-xs truncate max-w-[90px]" title={g}>{g}</span>
                          ))}
                          {groups.length > 2 && <span className="text-xs text-muted-foreground">+{groups.length - 2}</span>}
                        </div>
                        <span className="text-xs text-muted-foreground font-mono">{p.engine || 'openai'}</span>
                      </div>
                    </td>
                    <td className="px-4 py-3 text-center">
                      {totalKeys > 0 ? (
                        <span
                          className={`text-xs font-mono tabular-nums px-1.5 py-0.5 rounded ${
                            hasKeyIssue ? 'bg-amber-500/12 text-amber-600 dark:text-amber-400' : 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                          }`}
                          title={`可用: ${effectiveEnabled} / 总计: ${totalKeys}${configDisabledKeys > 0 ? ` (配置禁用: ${configDisabledKeys})` : ''}${rtDisabledCount > 0 ? ` (自动禁用: ${rtDisabledCount})` : ''}${rtCoolingCount > 0 ? ` (冷却中: ${rtCoolingCount})` : ''}`}
                        >
                          {effectiveEnabled}/{totalKeys}
                        </span>
                      ) : (
                        <span className="text-muted-foreground/50">—</span>
                      )}
                    </td>
                    <td className="px-4 py-3">
                      <div className="flex items-center gap-1.5 flex-wrap">
                        {modelLabels.length > 0 ? (
                          <span
                            className="inline-flex items-center gap-1 bg-secondary text-secondary-foreground px-1.5 py-0.5 rounded text-xs font-mono tabular-nums"
                            title={modelLabels.slice(0, 18).join(', ') + (modelLabels.length > 18 ? ` …共 ${modelLabels.length} 个模型` : '')}
                          >
                            <Boxes className="w-3 h-3 text-muted-foreground" />
                            {modelLabels.length}
                          </span>
                        ) : (
                          <span className="text-xs text-muted-foreground/50">0 模型</span>
                        )}
                        {plugins.length > 0 && (
                          <span
                            className="inline-flex items-center gap-1 bg-primary/10 text-primary px-1.5 py-0.5 rounded text-xs font-mono tabular-nums"
                            title={`已启用插件: ${plugins.join(', ')}`}
                          >
                            <Puzzle className="w-3 h-3" />
                            {plugins.length}
                          </span>
                        )}
                      </div>
                    </td>
                    <td className="px-4 py-3 text-center">
                      <button
                        type="button"
                        onClick={() => handleToggleProvider(idx)}
                        className={`inline-flex items-center justify-center w-6 h-6 rounded-full transition-colors ${isEnabled ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 hover:bg-emerald-500/20' : 'bg-muted text-muted-foreground hover:bg-muted/80'}`}
                        title={isEnabled ? '已启用（点击禁用）' : '已禁用（点击启用）'}
                      >
                        {isEnabled ? <CheckCircle2 className="w-4 h-4" /> : <X className="w-3.5 h-3.5" />}
                      </button>
                    </td>
                    <td className="px-4 py-3 text-center">
                      <input
                        type="number"
                        value={weight}
                        onChange={e => handleUpdateWeight(idx, parseInt(e.target.value) || 0)}
                        onClick={e => e.stopPropagation()}
                        className="w-14 bg-muted/70 border border-border rounded-md px-1.5 py-1 text-center font-mono tabular-nums text-xs text-foreground focus:border-primary outline-none"
                      />
                    </td>
                    <td className="px-4 py-3 text-right">
                      <div className="flex items-center justify-end gap-1">
                        <button
                          type="button"
                          onClick={() => openTestDialog(p)}
                          className="px-2 py-1 text-xs font-medium text-muted-foreground hover:text-primary hover:bg-primary/10 rounded-md transition-colors inline-flex items-center gap-1"
                          title="测试渠道连通性"
                        >
                          <Play className="w-3.5 h-3.5" />
                          <span className="hidden xl:inline">测试</span>
                        </button>
                        <button
                          type="button"
                          onClick={() => { setAnalyticsProvider(getProviderAnalyticsName(p)); setAnalyticsOpen(true); }}
                          className="p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors"
                          title="渠道统计分析"
                        >
                          <BarChart3 className="w-4 h-4" />
                        </button>
                        <button
                          type="button"
                          onClick={() => openModal(p, idx)}
                          className="p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors"
                          title="编辑渠道"
                        >
                          <Edit className="w-4 h-4" />
                        </button>
                        <DropdownMenu.Root>
                          <DropdownMenu.Trigger asChild>
                            <button
                              type="button"
                              className="p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors"
                              title="更多操作"
                              aria-label={`渠道 ${p.provider} 更多操作`}
                            >
                              <MoreHorizontal className="w-4 h-4" />
                            </button>
                          </DropdownMenu.Trigger>
                          <DropdownMenu.Portal>
                            <DropdownMenu.Content
                              align="end"
                              sideOffset={4}
                              className="min-w-[148px] bg-popover text-popover-foreground border border-border rounded-lg shadow-lg p-1 z-50 text-xs"
                            >
                              <DropdownMenu.Item
                                onSelect={() => handleToggleProvider(idx)}
                                className="flex items-center gap-2 px-2.5 py-1.5 rounded-md cursor-pointer outline-none hover:bg-muted"
                              >
                                <Power className="w-3.5 h-3.5 text-muted-foreground" />
                                {isEnabled ? '禁用渠道' : '启用渠道'}
                              </DropdownMenu.Item>
                              <DropdownMenu.Item
                                onSelect={() => handleCopyProvider(p)}
                                className="flex items-center gap-2 px-2.5 py-1.5 rounded-md cursor-pointer outline-none hover:bg-muted"
                              >
                                <Files className="w-3.5 h-3.5 text-muted-foreground" />
                                复制渠道配置
                              </DropdownMenu.Item>
                              <DropdownMenu.Separator className="my-1 h-px bg-border" />
                              <DropdownMenu.Item
                                onSelect={() => handleDeleteProvider(idx)}
                                className="flex items-center gap-2 px-2.5 py-1.5 rounded-md cursor-pointer outline-none text-destructive hover:bg-destructive/10"
                              >
                                <Trash2 className="w-3.5 h-3.5" />
                                删除渠道
                              </DropdownMenu.Item>
                            </DropdownMenu.Content>
                          </DropdownMenu.Portal>
                        </DropdownMenu.Root>
                      </div>
                    </td>
                  </tr>
                  {/* 子渠道二级行 */}
                  {(p.sub_channels || []).map((sub: any, subIdx: number) => {
                    const subEnabled = sub.enabled !== false;
                    const subModels = Array.isArray(sub.model) ? sub.model : Array.isArray(sub.models) ? sub.models : [];
                    const subModelCount = subModels.filter((m: any) => typeof m === 'string').length;
                    const subPlugins = sub.preferences?.enabled_plugins || [];
                    return (
                      <tr key={`${idx}-sub-${subIdx}`} className={`transition-colors bg-muted/20 ${!subEnabled && 'opacity-50'}`}>
                        <td className="px-4 py-2 pl-10" colSpan={1}>
                          <div className="flex items-center gap-2">
                            <span className="text-muted-foreground text-xs">└</span>
                            <span className="text-xs font-medium text-foreground">{sub.remark || sub.engine || '?'}</span>
                            <span className="text-[10px] text-muted-foreground">({subModelCount} 模型)</span>
                          </div>
                        </td>
                        <td className="px-4 py-2">
                          <span className="text-xs text-muted-foreground font-mono">{sub.remark || sub.engine || '-'}</span>
                        </td>
                        <td className="px-4 py-2 text-center">
                          <span className="text-xs text-muted-foreground">共享</span>
                        </td>
                        <td className="px-4 py-2">
                          {subPlugins.length > 0 ? (
                            <span className="bg-primary/10 text-primary px-1.5 py-0.5 rounded text-[10px]">{subPlugins.length}</span>
                          ) : <span className="text-[10px] text-muted-foreground">继承</span>}
                        </td>
                        <td className="px-4 py-2 text-center">
                          <span className={`inline-flex items-center justify-center w-5 h-5 rounded-full ${subEnabled ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-500' : 'bg-red-500/10 text-red-600 dark:text-red-500'}`}>
                            {subEnabled ? <CheckCircle2 className="w-3.5 h-3.5" /> : <X className="w-3.5 h-3.5" />}
                          </span>
                        </td>
                        <td className="px-4 py-2 text-center">
                          <span className="text-xs text-muted-foreground">{sub.preferences?.weight ?? '—'}</span>
                        </td>
                        <td className="px-4 py-2 text-right">
                          <div className="flex items-center justify-end gap-1">
                            <button onClick={() => { const sp = buildSubChannelProvider(idx, subIdx); if (sp) openTestDialog(sp); }} className="p-1 text-blue-600 dark:text-blue-400 hover:bg-blue-500/10 rounded-md transition-colors" title="测试子渠道">
                              <Play className="w-3.5 h-3.5" />
                            </button>
                            <button onClick={() => handleToggleSubChannel(idx, subIdx)} className={`p-1 rounded-md transition-colors ${subEnabled ? 'text-emerald-600 dark:text-emerald-500 hover:bg-emerald-500/10' : 'text-muted-foreground hover:bg-muted'}`} title={subEnabled ? '禁用' : '启用'}>
                              <Power className="w-3.5 h-3.5" />
                            </button>
                            <button onClick={() => openSubChannelEdit(idx, subIdx)} className="p-1 text-muted-foreground hover:text-foreground hover:bg-muted rounded-md transition-colors" title="编辑子渠道">
                              <Edit className="w-3.5 h-3.5" />
                            </button>
                            <button onClick={() => handleDeleteSubChannel(idx, subIdx)} className="p-1 text-red-600 dark:text-red-500 hover:bg-red-500/10 rounded-md transition-colors" title="删除子渠道">
                              <Trash2 className="w-3.5 h-3.5" />
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </Fragment>
                );
              });
              })()}
            </tbody>
          </table>
        )}
      </div>
      </div>
      </div>
    </>
  );
}
