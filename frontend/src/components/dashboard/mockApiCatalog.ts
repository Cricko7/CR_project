import type {
  EndpointCategory,
  EndpointDefinition,
  Graph3DEdge,
  Graph3DNode
} from './types';

const toJson = (value: unknown) => JSON.stringify(value, null, 2);
const DEMO_AGENT_A = '11111111-1111-1111-1111-111111111111';
const DEMO_AGENT_B = '22222222-2222-2222-2222-222222222222';
const DEMO_AGENT_C = '33333333-3333-4333-8333-333333333333';
const DEMO_AGENT_D = '44444444-4444-4444-8444-444444444444';
const DEMO_AGENT_E = '55555555-5555-4555-8555-555555555555';
const DEMO_AGENT_F = '66666666-6666-4666-8666-666666666666';
const DEMO_AGENT_G = '77777777-7777-4777-8777-777777777777';

export const CATEGORY_LABELS: Record<EndpointCategory, string> = {
  system: 'System',
  simulation: 'Simulation',
  events: 'Events & Interventions',
  agents: 'Agents',
  relationships: 'Relationships',
  memory: 'Memory',
  realtime: 'Realtime Streams'
};

export const API_ENDPOINTS: EndpointDefinition[] = [
  {
    id: 'health',
    title: 'Health Check',
    category: 'system',
    kind: 'rest',
    method: 'GET',
    path: '/health',
    summary: 'Service health probe.',
    sampleResponse: toJson({ status: 'ok', service: 'sim-backend' })
  },
  {
    id: 'livez',
    title: 'Liveness Check',
    category: 'system',
    kind: 'rest',
    method: 'GET',
    path: '/livez',
    summary: 'Container/runtime liveness probe.',
    sampleResponse: toJson({ status: 'ok', service: 'sim-backend' })
  },
  {
    id: 'agent-tick',
    title: 'Trigger Agent Tick',
    category: 'simulation',
    kind: 'rest',
    method: 'POST',
    path: '/agents/{id}/ticks',
    summary: 'Run one tick for an agent with idempotency support.',
    params: [{ key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true }],
    defaultBody: toJson({ tick_id: 'custom-idempotency-key' }),
    sampleResponse: toJson({
      outcome: 'applied',
      agent_id: 'uuid-agent-a',
      tick_id: 'custom-idempotency-key',
      event_id: 123,
      mood_label: 'neutral',
      valence: 0.03,
      arousal: -0.05
    })
  },
  {
    id: 'agent-state',
    title: 'Get Agent State',
    category: 'agents',
    kind: 'rest',
    method: 'GET',
    path: '/agents/{id}/state',
    summary: 'Current PAD mood state for selected agent.',
    params: [{ key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true }],
    sampleResponse: toJson({
      agent_id: 'uuid-agent-a',
      mood_label: 'calm',
      valence: 0.22,
      arousal: -0.04,
      updated_at: '2026-02-16T12:00:00Z'
    })
  },
  {
    id: 'agent-create',
    title: 'Create Agent',
    category: 'agents',
    kind: 'rest',
    method: 'POST',
    path: '/agents',
    summary: 'Create a new AI agent instance with optional avatar/personality.',
    defaultBody: toJson({
      name: 'New Agent',
      avatar_url: null,
      personality_json: {}
    }),
    sampleResponse: toJson({
      id: 'uuid-agent-z',
      name: 'New Agent',
      avatar_url: null,
      personality_json: {},
      created_at: '2026-02-17T12:00:00Z'
    })
  },
  {
    id: 'agent-inspector',
    title: 'Inspector Profile',
    category: 'agents',
    kind: 'rest',
    method: 'GET',
    path: '/agents/{id}/inspector',
    summary: 'Aggregated profile/state/events/messages/relationships/memory with recall.',
    params: [
      { key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true },
      { key: 'events_limit', label: 'Events', kind: 'query', defaultValue: '20' },
      { key: 'messages_limit', label: 'Messages', kind: 'query', defaultValue: '10' },
      { key: 'relationships_limit', label: 'Relationships', kind: 'query', defaultValue: '5' },
      { key: 'timeline_limit', label: 'Timeline', kind: 'query', defaultValue: '20' },
      { key: 'memories_limit', label: 'Memories', kind: 'query', defaultValue: '20' },
      { key: 'recall_query', label: 'Recall Query', kind: 'query', defaultValue: 'recent conflict' },
      { key: 'recall_top_k', label: 'Top-K', kind: 'query', defaultValue: '8' }
    ],
    sampleResponse: toJson({
      agent: { id: 'uuid-agent-a', name: 'Alice', avatar_url: null, created_at: '2026-02-16T12:00:00Z' },
      state: { agent_id: 'uuid-agent-a', mood_label: 'calm', valence: 0.22, arousal: -0.04 },
      summary: {
        events_count: 20,
        messages_count: 10,
        relationships_count: 5,
        timeline_count: 20,
        memories_count: 20
      },
      recent_events: [{ id: 1, event_type: 'agent.tick.executed' }],
      recent_messages: [{ id: 77, content: 'Hold position.' }],
      recent_relationships: [{ id: 5, affinity_score: 0.32 }],
      relationship_timeline: [{ message_id: 77, direction: 'outgoing' }],
      recent_memories: [{ id: 42, content: 'treasure map' }],
      recall: { query: 'recent conflict', top_k: 8, items: [{ memory_id: 42, score: 0.91 }] }
    })
  },
  {
    id: 'events',
    title: 'Events Feed',
    category: 'events',
    kind: 'rest',
    method: 'GET',
    path: '/events',
    summary: 'Live feed list with cursor polling support.',
    params: [
      { key: 'agent_id', label: 'Agent ID', kind: 'query', defaultValue: '' },
      { key: 'limit', label: 'Limit', kind: 'query', defaultValue: '50' },
      { key: 'after_id', label: 'After ID', kind: 'query', defaultValue: '' }
    ],
    sampleResponse: toJson({
      items: [
        {
          id: 1,
          agent_id: 'uuid-agent-a',
          event_type: 'agent.tick.executed',
          description: 'Agent Alice executed tick.',
          payload: '{"tick_id":"custom-idempotency-key"}',
          occurred_at: '2026-02-16T12:00:00Z'
        }
      ],
      next_after_id: 1
    })
  },
  {
    id: 'interventions-create',
    title: 'Create Intervention',
    category: 'events',
    kind: 'rest',
    method: 'POST',
    path: '/interventions',
    summary: 'Admin intervention action hub.',
    defaultBody: toJson({
      admin_user_id: 'demo-admin',
      action: {
        type: 'send_message',
        sender_agent_id: DEMO_AGENT_A,
        receiver_agent_id: DEMO_AGENT_B,
        content: 'Hold position and report status.'
      }
    }),
    sampleResponse: toJson({
      intervention: {
        id: 21,
        admin_user_id: 'demo-admin',
        action_type: 'send_message',
        result_status: 'applied',
        created_at: '2026-02-16T12:00:00Z'
      },
      effect: { type: 'message', message_id: 77, status: 'queued' }
    })
  },
  {
    id: 'interventions-list',
    title: 'List Interventions',
    category: 'events',
    kind: 'rest',
    method: 'GET',
    path: '/interventions',
    summary: 'History of moderation/intervention actions.',
    params: [{ key: 'limit', label: 'Limit', kind: 'query', defaultValue: '50' }],
    sampleResponse: toJson({
      items: [
        {
          id: 21,
          admin_user_id: 'demo-admin',
          action_type: 'send_message',
          result_status: 'applied',
          created_at: '2026-02-16T12:00:00Z'
        }
      ]
    })
  },
  {
    id: 'time-scale-get',
    title: 'Get Time Scale',
    category: 'simulation',
    kind: 'rest',
    method: 'GET',
    path: '/simulation/time-scale',
    summary: 'Current runtime simulation speed.',
    sampleResponse: toJson({
      time_scale: 1.0,
      updated_at: '2026-02-16T12:00:00Z'
    })
  },
  {
    id: 'time-scale-set',
    title: 'Set Time Scale',
    category: 'simulation',
    kind: 'rest',
    method: 'POST',
    path: '/simulation/time-scale',
    summary: 'Set runtime speed factor [0.1..10.0].',
    defaultBody: toJson({ time_scale: 1.5 }),
    sampleResponse: toJson({
      time_scale: 1.5,
      updated_at: '2026-02-16T12:05:00Z'
    })
  },
  {
    id: 'send-message',
    title: 'Send Agent Message',
    category: 'agents',
    kind: 'rest',
    method: 'POST',
    path: '/agents/{id}/messages',
    summary: 'Queue a direct message to receiver agent.',
    params: [{ key: 'id', label: 'Receiver ID', kind: 'path', defaultValue: 'uuid-agent-b', required: true }],
    defaultBody: toJson({
      sender_agent_id: DEMO_AGENT_A,
      content: "Let's cooperate on exploring the market."
    }),
    sampleResponse: toJson({ message_id: 77, status: 'queued' })
  },
  {
    id: 'list-messages',
    title: 'List Agent Messages',
    category: 'agents',
    kind: 'rest',
    method: 'GET',
    path: '/agents/{id}/messages',
    summary: 'Recent inbound/outbound messages for agent.',
    params: [
      { key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true },
      { key: 'limit', label: 'Limit', kind: 'query', defaultValue: '20' }
    ],
    sampleResponse: toJson({
      items: [
        {
          id: 77,
          sender_type: 'agent',
          sender_id: 'uuid-agent-a',
          receiver_agent_id: 'uuid-agent-b',
          content: "Let's cooperate on exploring the market.",
          status: 'delivered',
          created_at: '2026-02-16T12:00:00Z'
        }
      ]
    })
  },
  {
    id: 'list-relationships',
    title: 'List Relationships',
    category: 'relationships',
    kind: 'rest',
    method: 'GET',
    path: '/agents/{id}/relationships',
    summary: 'Affinity and relationship summaries for agent.',
    params: [
      { key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true },
      { key: 'limit', label: 'Limit', kind: 'query', defaultValue: '20' }
    ],
    sampleResponse: toJson({
      items: [
        {
          id: 5,
          agent_a: 'uuid-agent-a',
          agent_b: 'uuid-agent-b',
          affinity_score: 0.32,
          history_summary: "Let's cooperate on exploring the market.",
          last_interaction_at: '2026-02-16T12:00:01Z'
        }
      ]
    })
  },
  {
    id: 'relationship-history',
    title: 'Relationship Timeline',
    category: 'relationships',
    kind: 'rest',
    method: 'GET',
    path: '/agents/{id}/relationships/history',
    summary: 'Message-driven relationship history timeline.',
    params: [
      { key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true },
      { key: 'limit', label: 'Limit', kind: 'query', defaultValue: '50' }
    ],
    sampleResponse: toJson({
      agent_id: 'uuid-agent-a',
      items: [
        {
          message_id: 77,
          direction: 'outgoing',
          counterpart_agent_id: 'uuid-agent-b',
          counterpart_name: 'Bob',
          content: "Let's cooperate on exploring the market.",
          status: 'delivered',
          created_at: '2026-02-16T12:00:00Z',
          relationship: { id: 5, affinity_score: 0.32 }
        }
      ]
    })
  },
  {
    id: 'relationships-graph',
    title: 'Relationship Graph Snapshot',
    category: 'relationships',
    kind: 'rest',
    method: 'GET',
    path: '/relationships/graph',
    summary: 'Nodes and edges snapshot for relationship graph.',
    params: [
      { key: 'agent_id', label: 'Agent ID', kind: 'query', defaultValue: '' },
      { key: 'limit_edges', label: 'Limit Edges', kind: 'query', defaultValue: '100' }
    ],
    sampleResponse: toJson({
      nodes: [
        { agent_id: 'uuid-agent-a', name: 'Alice', avatar_url: null },
        { agent_id: 'uuid-agent-b', name: 'Bob', avatar_url: null }
      ],
      edges: [{ id: 5, agent_a: 'uuid-agent-a', agent_b: 'uuid-agent-b', affinity_score: 0.32 }]
    })
  },
  {
    id: 'memory-append',
    title: 'Append Memory',
    category: 'memory',
    kind: 'rest',
    method: 'POST',
    path: '/agents/{id}/memories',
    summary: 'Insert long-term memory entry.',
    params: [{ key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true }],
    defaultBody: toJson({ content: 'Agent found a treasure map', importance: 0.8 }),
    sampleResponse: toJson({ memory_id: 42, embedding_status: 'pending' })
  },
  {
    id: 'memory-recall',
    title: 'Recall Memory',
    category: 'memory',
    kind: 'rest',
    method: 'GET',
    path: '/agents/{id}/memories/recall',
    summary: 'Semantic memory recall endpoint.',
    params: [
      { key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true },
      { key: 'query', label: 'Query', kind: 'query', defaultValue: 'treasure map' },
      { key: 'top_k', label: 'Top K', kind: 'query', defaultValue: '8' }
    ],
    sampleResponse: toJson({
      items: [
        {
          memory_id: 42,
          score: 0.91,
          content: 'Agent found a treasure map',
          summary: null,
          importance: 0.8,
          created_at: '2026-02-16T12:00:00Z'
        }
      ]
    })
  },
  {
    id: 'memory-summarize',
    title: 'Summarize Memories',
    category: 'memory',
    kind: 'rest',
    method: 'POST',
    path: '/agents/{id}/memories/summarize',
    summary: 'Manual summarization of memory entries.',
    params: [{ key: 'id', label: 'Agent ID', kind: 'path', defaultValue: 'uuid-agent-a', required: true }],
    defaultBody: toJson({ max_active: 200, batch_size: 20 }),
    sampleResponse: toJson({ created_summary: true, source_count: 20, summary_entry_id: 1001 })
  },
  {
    id: 'memory-process-embeddings',
    title: 'Process Embeddings',
    category: 'memory',
    kind: 'rest',
    method: 'POST',
    path: '/memory/process-embeddings',
    summary: 'Process pending memory embeddings in batch.',
    defaultBody: toJson({ limit: 50 }),
    sampleResponse: toJson({
      processed: 10,
      succeeded: 8,
      failed: 2,
      retried: 1,
      dead_lettered: 1
    })
  },
  {
    id: 'memory-dead-letter',
    title: 'Dead-Letter List',
    category: 'memory',
    kind: 'rest',
    method: 'GET',
    path: '/memory/dead-letter',
    summary: 'List failed embeddings that require replay.',
    params: [{ key: 'limit', label: 'Limit', kind: 'query', defaultValue: '50' }],
    sampleResponse: toJson({
      items: [
        {
          memory_id: 42,
          agent_id: 'uuid-agent-a',
          content: 'Agent found a treasure map',
          importance: 0.8,
          embedding_status: 'dead_letter'
        }
      ]
    })
  },
  {
    id: 'memory-requeue',
    title: 'Requeue Dead-Letter',
    category: 'memory',
    kind: 'rest',
    method: 'POST',
    path: '/memory/dead-letter/{memory_id}/requeue',
    summary: 'Requeue single dead-letter memory embedding.',
    params: [{ key: 'memory_id', label: 'Memory ID', kind: 'path', defaultValue: '42', required: true }],
    sampleResponse: toJson({ memory_id: 42, requeued: true })
  },
  {
    id: 'ws-events',
    title: 'Events Stream',
    category: 'realtime',
    kind: 'ws',
    method: 'GET',
    path: '/ws/events',
    summary: 'Snapshot + live event_appended/tick_skipped/error stream.',
    params: [
      { key: 'agent_id', label: 'Agent ID', kind: 'query', defaultValue: '' },
      { key: 'snapshot_limit', label: 'Snapshot Limit', kind: 'query', defaultValue: '50' }
    ],
    wsEvents: ['snapshot', 'event_appended', 'tick_skipped', 'error'],
    sampleResponse: toJson({
      type: 'event_appended',
      item: {
        id: 123,
        agent_id: 'uuid-agent-a',
        event_type: 'agent.tick.executed',
        description: 'Agent Alice executed tick ...'
      }
    })
  },
  {
    id: 'ws-relationships',
    title: 'Relationships Stream',
    category: 'realtime',
    kind: 'ws',
    method: 'GET',
    path: '/ws/relationships',
    summary: 'Snapshot + edge_updated/error stream for graph updates.',
    params: [
      { key: 'agent_id', label: 'Agent ID', kind: 'query', defaultValue: '' },
      { key: 'snapshot_limit', label: 'Snapshot Limit', kind: 'query', defaultValue: '100' }
    ],
    wsEvents: ['snapshot', 'edge_updated', 'error'],
    sampleResponse: toJson({
      type: 'edge_updated',
      edge: {
        id: 5,
        agent_a: 'uuid-agent-a',
        agent_b: 'uuid-agent-b',
        affinity_score: 0.4,
        history_summary: 'Lets cooperate | Thanks for support'
      }
    })
  }
];

export const RELATIONSHIP_GRAPH_3D_NODES: Graph3DNode[] = [
  { id: DEMO_AGENT_A, label: 'Alice', x: -90, y: -30, z: 90 },
  { id: DEMO_AGENT_B, label: 'Bob', x: 110, y: -25, z: -70 },
  { id: DEMO_AGENT_C, label: 'Eve', x: -45, y: 90, z: -35 },
  { id: DEMO_AGENT_D, label: 'Milo', x: 75, y: 78, z: 50 },
  { id: DEMO_AGENT_E, label: 'Nova', x: 15, y: -95, z: -95 }
];

export const AGENT_DIRECTORY = [
  ...RELATIONSHIP_GRAPH_3D_NODES.map((node) => ({ id: node.id, name: node.label })),
  { id: DEMO_AGENT_F, name: 'Iris' },
  { id: DEMO_AGENT_G, name: 'Orion' }
];

export const RELATIONSHIP_GRAPH_3D_EDGES: Graph3DEdge[] = [
  { id: 'edge-1', source: DEMO_AGENT_A, target: DEMO_AGENT_B, affinity: 0.32 },
  { id: 'edge-2', source: DEMO_AGENT_A, target: DEMO_AGENT_C, affinity: 0.74 },
  { id: 'edge-3', source: DEMO_AGENT_C, target: DEMO_AGENT_D, affinity: -0.23 },
  { id: 'edge-4', source: DEMO_AGENT_B, target: DEMO_AGENT_D, affinity: 0.58 },
  { id: 'edge-5', source: DEMO_AGENT_E, target: DEMO_AGENT_A, affinity: 0.19 },
  { id: 'edge-6', source: DEMO_AGENT_E, target: DEMO_AGENT_D, affinity: -0.11 }
];
