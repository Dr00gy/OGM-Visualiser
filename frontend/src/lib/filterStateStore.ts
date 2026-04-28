import { writable } from 'svelte/store';
import { browser } from '$app/environment';

export interface DonutFltState {
  /** Selected query sequence id ('' = no filter). */
  selSeqId: string;
  selGen1: string;
  selGen2: string;
  /** Chromosome filter (1..24 as string; '' = none). */
  selChr: string;
  /** Which genome the chromosome filter applies to. */
  selGenForChr: string;
  /** Render intra-genome flows instead of cross-genome ones. */
  showDups: boolean;
  /** Donut zoom scale. */
  scale: number;
}

export interface AreaFltState {
  /** Selected genome indices. */
  selFiles: number[];
  /** Chromosome being inspected (1..24). */
  selChr: number;
  /** Window size in base pairs. */
  winSize: number;
  /** Zero-based index of the displayed window. */
  curWinIdx: number;
  /** Search box text. */
  qry: string;
}

const DONUT_DEF: DonutFltState = {
  selSeqId: '',
  selGen1: '',
  selGen2: '',
  selChr: '',
  selGenForChr: '',
  showDups: false,
  scale: 1.0,
};

const AREA_DEF: AreaFltState = {
  selFiles: [0],
  selChr: 1,
  winSize: 100000,
  curWinIdx: 0,
  qry: '',
};

const AREA_LS_KEY = 'areaFltSt';

function makeDonutFltStore() {
  const { subscribe, set, update } = writable<DonutFltState>({ ...DONUT_DEF });
  return { subscribe, set, update, reset: () => set({ ...DONUT_DEF }) };
}

function makeAreaFltStore() {
  const init = (): AreaFltState => {
    if (!browser) return { ...AREA_DEF };
    const stored = localStorage.getItem(AREA_LS_KEY);
    if (stored) {
      try { return JSON.parse(stored); }
      catch (e) { console.error('Failed to parse stored filter state:', e); }
    }
    return { ...AREA_DEF };
  };

  const persist = (v: AreaFltState) => {
    if (browser) localStorage.setItem(AREA_LS_KEY, JSON.stringify(v));
  };

  const { subscribe, set, update } = writable<AreaFltState>(init());

  return {
    subscribe,
    set: (v: AreaFltState) => { persist(v); set(v); },
    update: (fn: (s: AreaFltState) => AreaFltState) => {
      update(s => { const n = fn(s); persist(n); return n; });
    },
    reset: () => { persist(AREA_DEF); set({ ...AREA_DEF }); },
  };
}

export const donutFltState = makeDonutFltStore();
export const areaFltState = makeAreaFltStore();