import { writable } from 'svelte/store';

interface SearchState {
  /** Donut overview list search box. */
  ovQry: string;
  /** Donut match-table search box. */
  mtcQry: string;
  /** Area-analysis search box. */
  areaQry: string;
  ovType: 'sequence' | 'chromosome' | 'confidence';
  mtcType: 'sequence' | 'chromosome' | 'confidence';
}

const initSt: SearchState = {
  ovQry: '',
  mtcQry: '',
  areaQry: '',
  ovType: 'sequence',
  mtcType: 'sequence'
};

export const searchStore = writable<SearchState>(initSt);
export function resetSearchStore() {
  searchStore.set(initSt);
}