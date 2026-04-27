import { writable } from 'svelte/store';

interface SearchState {
  /** Text typed into the donut overview list's search box. */
  ovQry: string;
  /** Text typed into the donut match-table's search box. */
  mtcQry: string;
  /** Text typed into the area-analysis tab's search box. */
  areaQry: string;
  /** Which field the overview search is being applied to. */
  ovType: 'sequence' | 'chromosome' | 'confidence';
  /** Which field the match-table search is being applied to. */
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