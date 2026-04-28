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

const AREA_LS_KEY = 'areaFltSt';

function makeDonutFltStore() {
  const def: DonutFltState = {
    selSeqId: '',
    selGen1: '',
    selGen2: '',
    selChr: '',
    selGenForChr: '',
    showDups: false,
    scale: 1.0
  };

  const { subscribe, set, update } = writable<DonutFltState>(def);

  return {
    subscribe,
    set,
    update,
    reset: () => set(def)
  };
}

function makeAreaFltStore() {
  const init = (): AreaFltState => {
    if (!browser) {
      return {
        selFiles: [0],
        selChr: 1,
        winSize: 100000,
        curWinIdx: 0,
        qry: ''
      };
    }

    const stored = localStorage.getItem(AREA_LS_KEY);
    if (stored) {
      try {
        return JSON.parse(stored);
      } catch (e) {
        console.error('Failed to parse stored filter state:', e);
      }
    }

    return {
      selFiles: [0],
      selChr: 1,
      winSize: 100000,
      curWinIdx: 0,
      qry: ''
    };
  };

  const { subscribe, set, update } = writable<AreaFltState>(init());

  return {
    subscribe,

    set: (v: AreaFltState) => {
      if (browser) {
        localStorage.setItem(AREA_LS_KEY, JSON.stringify(v));
      }
      set(v);
    },

    update: (fn: (s: AreaFltState) => AreaFltState) => {
      update((s) => {
        const next = fn(s);
        if (browser) {
          localStorage.setItem(AREA_LS_KEY, JSON.stringify(next));
        }
        return next;
      });
    },

    reset: () => {
      const def: AreaFltState = {
        selFiles: [0],
        selChr: 1,
        winSize: 100000,
        curWinIdx: 0,
        qry: ''
      };
      if (browser) {
        localStorage.setItem(AREA_LS_KEY, JSON.stringify(def));
      }
      set(def);
    }
  };
}

export const donutFltState = makeDonutFltStore();
export const areaFltState = makeAreaFltStore();