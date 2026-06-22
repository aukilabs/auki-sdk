export type NodeType = "file" | "function" | "class" | "module" | "concept" | "config" | "document" | "service" | "table" | "endpoint" | "pipeline" | "schema" | "resource" | "domain" | "flow" | "step" | "article" | "entity" | "topic" | "claim" | "source";
export type EdgeType = "imports" | "exports" | "contains" | "inherits" | "implements" | "calls" | "subscribes" | "publishes" | "middleware" | "reads_from" | "writes_to" | "transforms" | "validates" | "depends_on" | "tested_by" | "configures" | "related" | "similar_to" | "deploys" | "serves" | "provisions" | "triggers" | "migrates" | "documents" | "routes" | "defines_schema" | "contains_flow" | "flow_step" | "cross_domain" | "cites" | "contradicts" | "builds_on" | "exemplifies" | "categorized_under" | "authored_by";
export interface KnowledgeMeta {
    wikilinks?: string[];
    backlinks?: string[];
    category?: string;
    content?: string;
}
export interface DomainMeta {
    entities?: string[];
    businessRules?: string[];
    crossDomainInteractions?: string[];
    entryPoint?: string;
    entryType?: "http" | "cli" | "event" | "cron" | "manual";
}
export interface GraphNode {
    id: string;
    type: NodeType;
    name: string;
    filePath?: string;
    lineRange?: [number, number];
    summary: string;
    tags: string[];
    complexity: "simple" | "moderate" | "complex";
    languageNotes?: string;
    domainMeta?: DomainMeta;
    knowledgeMeta?: KnowledgeMeta;
}
export interface GraphEdge {
    source: string;
    target: string;
    type: EdgeType;
    direction: "forward" | "backward" | "bidirectional";
    description?: string;
    weight: number;
}
export interface Layer {
    id: string;
    name: string;
    description: string;
    nodeIds: string[];
}
export interface TourStep {
    order: number;
    title: string;
    description: string;
    nodeIds: string[];
    languageLesson?: string;
}
export interface ProjectMeta {
    name: string;
    languages: string[];
    frameworks: string[];
    description: string;
    analyzedAt: string;
    gitCommitHash: string;
}
export interface KnowledgeGraph {
    version: string;
    kind?: "codebase" | "knowledge";
    project: ProjectMeta;
    nodes: GraphNode[];
    edges: GraphEdge[];
    layers: Layer[];
    tour: TourStep[];
}
export interface ThemeConfig {
    presetId: string;
    accentId: string;
}
export interface AnalysisMeta {
    lastAnalyzedAt: string;
    gitCommitHash: string;
    version: string;
    analyzedFiles: number;
    theme?: ThemeConfig;
}
export interface ProjectConfig {
    autoUpdate: boolean;
    outputLanguage?: string;
}
export interface SectionInfo {
    name: string;
    level: number;
    lineRange: [number, number];
}
export interface DefinitionInfo {
    name: string;
    /** Parser-reported definition kind. Known values: "table", "view", "index", "message", "enum", "type", "input", "interface", "union", "scalar", "variable", "output", "resource", "data", "section", "target", "stage" */
    kind: string;
    lineRange: [number, number];
    fields: string[];
}
export interface ServiceInfo {
    name: string;
    image?: string;
    ports: number[];
    lineRange?: [number, number];
}
export interface EndpointInfo {
    method?: string;
    path: string;
    lineRange: [number, number];
}
export interface StepInfo {
    name: string;
    lineRange: [number, number];
}
export interface ResourceInfo {
    name: string;
    kind: string;
    lineRange: [number, number];
}
export interface ReferenceResolution {
    source: string;
    target: string;
    referenceType: string;
    line?: number;
}
export interface StructuralAnalysis {
    functions: Array<{
        name: string;
        lineRange: [number, number];
        params: string[];
        returnType?: string;
    }>;
    classes: Array<{
        name: string;
        lineRange: [number, number];
        methods: string[];
        properties: string[];
    }>;
    imports: Array<{
        source: string;
        specifiers: string[];
        lineNumber: number;
    }>;
    exports: Array<{
        name: string;
        lineNumber: number;
        isDefault?: boolean;
    }>;
    sections?: SectionInfo[];
    definitions?: DefinitionInfo[];
    services?: ServiceInfo[];
    endpoints?: EndpointInfo[];
    steps?: StepInfo[];
    resources?: ResourceInfo[];
}
export interface ImportResolution {
    source: string;
    resolvedPath: string;
    specifiers: string[];
}
export interface CallGraphEntry {
    caller: string;
    callee: string;
    lineNumber: number;
}
export interface AnalyzerPlugin {
    name: string;
    languages: string[];
    analyzeFile(filePath: string, content: string): StructuralAnalysis;
    resolveImports?(filePath: string, content: string): ImportResolution[];
    extractCallGraph?(filePath: string, content: string): CallGraphEntry[];
    extractReferences?(filePath: string, content: string): ReferenceResolution[];
}
//# sourceMappingURL=types.d.ts.map