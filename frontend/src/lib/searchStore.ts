import { writable } from 'svelte/store';

interface SearchState {
  /** Text typed into the donut overview list's search box. */
  overviewSearchQuery: string;
  /** Text typed into the donut match-table's search box. */
  matchesSearchQuery: string;
  /** Text typed into the area-analysis tab's search box. */
  areaSearchQuery: string;
  /** Which field the overview search is being applied to. */
  overviewSearchType: 'contig' | 'chromosome' | 'confidence';
  /** Which field the match-table search is being applied to. */
  matchesSearchType: 'contig' | 'chromosome' | 'confidence';
}

const initialState: SearchState = {
  overviewSearchQuery: '',
  matchesSearchQuery: '',
  areaSearchQuery: '',
  overviewSearchType: 'contig',
  matchesSearchType: 'contig'
};

export const searchStore = writable<SearchState>(initialState);
export function resetSearchStore() {
  searchStore.set(initialState);
}