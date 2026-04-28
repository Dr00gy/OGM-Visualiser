import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/svelte';
import DonutInfo from '$routes/DonutInfo.svelte';
import type { FileData, DonutSeg } from '$lib/types';

// Avoid network calls for the genome / match list when the component
// runs effects on mount.
vi.mock('$lib/queryClient', () => ({
  fetchSeqs:        vi.fn().mockResolvedValue({ total: 0, items: [] }),
  fetchMatchesPage: vi.fn().mockResolvedValue({ total: 0, items: [] }),
  makeDebouncer:    () => {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    
    // Create a function that can be called and also has a .cancel property
    const debouncer = (fn: () => void) => {
      if (timeoutId) clearTimeout(timeoutId);
      timeoutId = setTimeout(fn, 0);
    };
    
    debouncer.cancel = () => {
      if (timeoutId) clearTimeout(timeoutId);
      timeoutId = null;
    };
    
    return debouncer;
  },
}));

const mkFiles = (n: number): FileData[] =>
  Array.from({ length: n }, (_, i) => ({
    name: `genome${i + 1}.cmap`,
    rows: 100 * (i + 1),
    color: ['#3b82f6', '#10b981', '#f59e0b'][i] ?? '#888',
  }));

const baseProps = {
  segments: [] as DonutSeg[],
  genSizes: new Map<number, number>(),
  totGenSize: 0,
  fltFlowPaths: [],
  availSeqIds: [],
  availGens: [],
  availChrs: [],
  sessId: null,
  isQueryable: false,
};

describe('DonutInfo', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders the genome count taken from the files prop', () => {
    const { getByText } = render(DonutInfo, {
      props: { ...baseProps, files: mkFiles(2), fileToGen: [0, 1] },
    });
    expect(getByText(/Genomes \(2\)/)).toBeTruthy();
  });

  it('updates the genome count when the files prop changes', () => {
    const r1 = render(DonutInfo, {
      props: { ...baseProps, files: mkFiles(2), fileToGen: [0, 1] },
    });
    expect(r1.getByText(/Genomes \(2\)/)).toBeTruthy();
    r1.unmount();

    const r2 = render(DonutInfo, {
      props: { ...baseProps, files: mkFiles(3), fileToGen: [0, 1, 2] },
    });
    expect(r2.getByText(/Genomes \(3\)/)).toBeTruthy();
    expect(r2.queryByText(/Genomes \(2\)/)).toBeNull();
  });

  it('renders zero genomes when files is empty', () => {
    const { getByText } = render(DonutInfo, {
      props: { ...baseProps, files: [], fileToGen: [] },
    });
    expect(getByText(/Genomes \(0\)/)).toBeTruthy();
  });

  it('reflects total genome size and flow-path count in the summary', () => {
    const { getAllByText } = render(DonutInfo, {
      props: {
        ...baseProps,
        files: mkFiles(2),
        fileToGen: [0, 1],
        totGenSize: 6_000_000_000,
        fltFlowPaths: [{}, {}, {}],
      },
    });

    expect(getAllByText(/6[\s,.\u202f]?000[\s,.\u202f]?000[\s,.\u202f]?000/)).toHaveLength(1);

    // Match any element whose text includes "Flow Paths: 3"
    const matches = getAllByText((_content: string, el: Element | null) =>
      (el?.textContent ?? '').replace(/\s+/g, ' ').includes('Flow Paths: 3')
    );
    expect(matches.length).toBeGreaterThan(0);
  });
});