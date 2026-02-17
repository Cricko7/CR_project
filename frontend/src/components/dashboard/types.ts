export type HttpMethod = 'GET' | 'POST';
export type EndpointKind = 'rest' | 'ws';

export type EndpointCategory =
  | 'system'
  | 'simulation'
  | 'events'
  | 'agents'
  | 'relationships'
  | 'memory'
  | 'realtime';

export interface EndpointParam {
  key: string;
  label: string;
  kind: 'path' | 'query';
  defaultValue: string;
  required?: boolean;
}

export interface EndpointDefinition {
  id: string;
  title: string;
  category: EndpointCategory;
  kind: EndpointKind;
  method: HttpMethod;
  path: string;
  summary: string;
  params?: EndpointParam[];
  defaultBody?: string;
  sampleResponse: string;
  wsEvents?: string[];
}

export interface Graph3DNode {
  id: string;
  label: string;
  x: number;
  y: number;
  z: number;
}

export interface Graph3DEdge {
  id: string;
  source: string;
  target: string;
  affinity: number;
}
