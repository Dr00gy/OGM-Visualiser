import { writable } from 'svelte/store';
import { browser } from '$app/environment';

// ---------------------------------------------------------------------------
// Donut tab
// ---------------------------------------------------------------------------

export interface DonutFilterState {
  /** Selected query contig ID (as string; '' = no filter). */
  selectedQueryContigId: string;
  selectedGenome1: string;
  selectedGenome2: string;
  /** Chromosome filter (1..24 as string; '' = none). */
  selectedChromosome: string;
  /** Which genome the chromosome filter applies to. */
  selectedGenomeForChromosome: string;
  /** Render intra-genome flows instead of cross-genome ones. */
  showDuplicates: boolean;
  /** Donut zoom scale (1.0 = default). */
  scale: number;
}

// ---------------------------------------------------------------------------
// Area-analysis tab
// ---------------------------------------------------------------------------

export interface AreaAnalysisFilterState {
  /** Selected GENOME indices. */
  selectedFiles: number[];
  /** Chromosome currently being inspected (1..24). */
  selectedChromosome: number;
  /** Window size in base pairs. Default 100 kb. */
  windowSize: number;
  /** Zero-based index of the currently-displayed window. */
  currentWindowIndex: number;
  /** text in the search box. */
  searchQuery: string;
}

function createDonutFilterStateStore() {
  const defaultState: DonutFilterState = {
    selectedQueryContigId: '',
    selectedGenome1: '',
    selectedGenome2: '',
    selectedChromosome: '',
    selectedGenomeForChromosome: '',
    showDuplicates: false,
    scale: 1.0
  };

  const { subscribe, set, update } = writable<DonutFilterState>(defaultState);

  return {
    subscribe,
    set,
    update,
    reset: () => set(defaultState)
  };
}

function createAreaAnalysisFilterStateStore() { // TODO:
  const getInitialValue = (): AreaAnalysisFilterState => {
    if (!browser) {
      return {
        selectedFiles: [0],
        selectedChromosome: 1,
        windowSize: 100000,
        currentWindowIndex: 0,
        searchQuery: ''
      };
    }

    const stored = localStorage.getItem('areaAnalysisFilterState');
    if (stored) {
      try {
        return JSON.parse(stored);
      } catch (e) {
        console.error('Failed to parse stored filter state:', e);
      }
    }

    return {
      selectedFiles: [0],
      selectedChromosome: 1,
      windowSize: 100000,
      currentWindowIndex: 0,
      searchQuery: ''
    };
  };

  const { subscribe, set, update } = writable<AreaAnalysisFilterState>(getInitialValue());

  return {
    subscribe,

    set: (value: AreaAnalysisFilterState) => {
      if (browser) {
        localStorage.setItem('areaAnalysisFilterState', JSON.stringify(value));
      }
      set(value);
    },

    update: (updater: (state: AreaAnalysisFilterState) => AreaAnalysisFilterState) => {
      update((state) => {
        const newState = updater(state);
        if (browser) {
          localStorage.setItem('areaAnalysisFilterState', JSON.stringify(newState));
        }
        return newState;
      });
    },

    reset: () => {
      const defaultState: AreaAnalysisFilterState = {
        selectedFiles: [0],
        selectedChromosome: 1,
        windowSize: 100000,
        currentWindowIndex: 0,
        searchQuery: ''
      };
      if (browser) {
        localStorage.setItem('areaAnalysisFilterState', JSON.stringify(defaultState));
      }
      set(defaultState);
    }
  };
}

export const donutFilterState = createDonutFilterStateStore();
export const areaAnalysisFilterState = createAreaAnalysisFilterStateStore();