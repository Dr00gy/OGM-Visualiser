export const mockMatchedRecords = [
  {
    file_index: 0,
    ref_contig_id: 1,
    qry_start_pos: 0,
    qry_end_pos: 5000,
    ref_start_pos: 0,
    ref_end_pos: 250000,
    orientation: '+',
    confidence: 9.8,
    ref_len: 250000,
  },
  {
    file_index: 1,
    ref_contig_id: 2,
    qry_start_pos: 2000,
    qry_end_pos: 8000,
    ref_start_pos: 0,
    ref_end_pos: 300000,
    orientation: '-',
    confidence: 8.2,
    ref_len: 300000,
  },
];

export const mockBackendMatch = {
  qry_contig_id: 2001,
  file_indices: [0, 1],
  records: mockMatchedRecords,
};