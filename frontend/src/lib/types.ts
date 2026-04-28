export interface FileData {
  name: string;
  rows: number;
  color: string;
}

export interface DonutSeg {
  name: string;
  rows: number;
  color: string;
  idx: number;
  genSize: number;
  dashArray: string;
  dashOffset: number;
  pct: string;
  showLabel: boolean;
  showChrs: boolean;
  startAng: number;
  endAng: number;
  angRange: number;
}