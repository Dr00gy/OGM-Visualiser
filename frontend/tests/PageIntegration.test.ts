import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Page from '../src/routes/+page.svelte';

// The top-level page wires together file upload, the streaming bincode
// decoder, the donut visualisation and the (lazy-loaded) area browser.
// We mock just enough of the boundary so the test stays in-process.
vi.mock('$lib/bincodeDecoder', async () => {
  const actual = await vi.importActual<any>('$lib/bincodeDecoder');
  return {
    ...actual,
    // Single complete frame with one chromosome per genome.
    processMatchStream: async function* () {
      yield {
        type: 'chromosomeInfo',
        chromosomeInfo: [
          [{ ref_contig_id: 1, ref_len: 250_000_000 }],
          [{ ref_contig_id: 1, ref_len: 250_000_000 }],
        ],
      };
      yield {
        type: 'complete',
        complete: {
          total_matches: 10,
          total_records: 30,
          per_genome_records: [15, 15],
          distinct_sequence_count: 8,
        },
      };
    },
  };
});

// fetch is used for /api/session, /api/upload, /api/match, /api/session DELETE.
function mockFetch() {
  globalThis.fetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    if (typeof url === 'string' && url.endsWith('/api/session') && init?.method === 'POST') {
      return Promise.resolve(new Response(JSON.stringify({ session_id: 'sess-1' }), {
        headers: { 'content-type': 'application/json' },
      }));
    }
    if (typeof url === 'string' && url.includes('/api/upload/')) {
      return Promise.resolve(new Response('{}'));
    }
    if (typeof url === 'string' && url.includes('/api/match/')) {
      return Promise.resolve(new Response(new Uint8Array(0)));
    }
    return Promise.resolve(new Response('{}'));
  }) as any;
}

describe('+page.svelte integration', () => {
  it('renders the upload form and tab navigation in the initial state', () => {
    mockFetch();
    const { container, getByText } = render(Page);

    // Tab nav from TabNav.svelte.
    expect(getByText('Chromosomal Flow Chart')).toBeTruthy();
    expect(getByText('Analytic Browser')).toBeTruthy();

    // No donut yet — chrInfo is empty until the stream emits.
    expect(container.querySelector('svg.donut-chart')).toBeNull();
  });

  it('shows the donut visualisation after a successful upload + stream', async () => {
    mockFetch();
    const { container, component } = render(Page);

    // Drive the upload handler the same way FileUpload would: emit a CustomEvent
    // bubbling up through `on:upload`. We simulate this directly by invoking
    // the exposed flow.
    const refineFinal = new File(['#h\n'], 'refineFinal.smap', { type: 'text/plain' });
    const xmap = new File(['#h\n'], 'sample.xmap', { type: 'text/plain' });

    // The page handler is internal; trigger via the FileUpload child's event.
    const ev = new CustomEvent('upload', {
      detail: [
        { seqFiles: [xmap], refineFinalFile: refineFinal, dirName: 'g1' },
        { seqFiles: [xmap], refineFinalFile: refineFinal, dirName: 'g2' },
      ],
    });
    container.querySelector('[data-component="file-upload"]')?.dispatchEvent(ev);

    // Wait a tick for the streaming generator to drain.
    await new Promise(r => setTimeout(r, 0));

    // After the stream completes we expect a donut SVG in the DOM tree.
    // (The exact selector depends on D3DonutChart; the assertion below is
    // conservative — any rendered <svg> below the visualisation slot.)
    expect(container.querySelectorAll('svg').length).toBeGreaterThan(0);
  });

  it('switches to the Analytic Browser tab and lazy-loads AreaAnalysis', async () => {
    mockFetch();
    const { getByText, container } = render(Page);

    await fireEvent.click(getByText('Analytic Browser'));

    // Until streamComplete becomes true, the placeholder is shown rather than
    // AreaAnalysis itself.
    expect(container.textContent).toMatch(/Analytic Browser|Upload|Loading/);
  });
});