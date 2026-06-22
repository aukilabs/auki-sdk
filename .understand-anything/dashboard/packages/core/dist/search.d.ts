import type { GraphNode } from "./types.js";
export interface SearchResult {
    nodeId: string;
    score: number;
}
export interface SearchOptions {
    types?: GraphNode["type"][];
    limit?: number;
}
export declare class SearchEngine {
    private fuse;
    private nodes;
    constructor(nodes: GraphNode[]);
    search(query: string, options?: SearchOptions): SearchResult[];
    updateNodes(nodes: GraphNode[]): void;
}
//# sourceMappingURL=search.d.ts.map