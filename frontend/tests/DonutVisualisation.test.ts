import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/svelte';
import DonutVisualisation from '$routes/DonutVisualisation.svelte';
import type { ChromosomeInfo } from '$lib/bincodeDecoder';
import type { FileData } from '$lib/types';

const mkFiles = (): FileData[] => [
  { name: 'A.cmap', rows: 100, color: '#3b82f6' },
  { name: 'B.cmap', rows: 200, color: '#10b981' },
];

const mkChrInfo = (): ChromosomeInfo[][] => [
  [{ ref_contig_id: 1, ref_len: 250_000_000 }],
  [{ ref_contig_id: 1, ref_len: 250_000_000 }],
];

describe('DonutVisualisation', () => {
  it('mounts with valid props', () => {
    const { container } = render(DonutVisualisation, {
      props: {
        files: mkFiles(),
        fileToGen: [0, 1],
        chrInfo: mkChrInfo(),
        showDups: true,
        isStreaming: true,
        sessId: 'session-abc',
        isQueryable: false,
      },
    });

    // Just verify the component rendered.
    expect(container.firstElementChild).not.toBeNull();
  });

  it('mounts with default/empty props', () => {
    const { container } = render(DonutVisualisation, {
      props: {},
    });

    expect(container.firstElementChild).not.toBeNull();
  });

  it('renders and accepts all expected props without error', () => {
    expect(() => {
      render(DonutVisualisation, {
        props: {
          files: mkFiles(),
          fileToGen: [0, 1],
          chrInfo: mkChrInfo(),
          showDups: true,
          isStreaming: true,
          sessId: 'session-abc',
          isQueryable: false,
        },
      });
    }).not.toThrow();
  });
});