import { z } from "zod";
export declare const EdgeTypeSchema: z.ZodEnum<{
    imports: "imports";
    exports: "exports";
    contains: "contains";
    inherits: "inherits";
    implements: "implements";
    calls: "calls";
    subscribes: "subscribes";
    publishes: "publishes";
    middleware: "middleware";
    reads_from: "reads_from";
    writes_to: "writes_to";
    transforms: "transforms";
    validates: "validates";
    depends_on: "depends_on";
    tested_by: "tested_by";
    configures: "configures";
    related: "related";
    similar_to: "similar_to";
    deploys: "deploys";
    serves: "serves";
    provisions: "provisions";
    triggers: "triggers";
    migrates: "migrates";
    documents: "documents";
    routes: "routes";
    defines_schema: "defines_schema";
    contains_flow: "contains_flow";
    flow_step: "flow_step";
    cross_domain: "cross_domain";
    cites: "cites";
    contradicts: "contradicts";
    builds_on: "builds_on";
    exemplifies: "exemplifies";
    categorized_under: "categorized_under";
    authored_by: "authored_by";
}>;
export declare const NODE_TYPE_ALIASES: Record<string, string>;
export declare const EDGE_TYPE_ALIASES: Record<string, string>;
export declare const COMPLEXITY_ALIASES: Record<string, string>;
export declare const DIRECTION_ALIASES: Record<string, string>;
export declare function sanitizeGraph(data: Record<string, unknown>): Record<string, unknown>;
export declare function autoFixGraph(data: Record<string, unknown>): {
    data: Record<string, unknown>;
    issues: GraphIssue[];
};
export declare const GraphNodeSchema: z.ZodObject<{
    id: z.ZodString;
    type: z.ZodEnum<{
        function: "function";
        file: "file";
        class: "class";
        module: "module";
        concept: "concept";
        config: "config";
        document: "document";
        service: "service";
        table: "table";
        endpoint: "endpoint";
        pipeline: "pipeline";
        schema: "schema";
        resource: "resource";
        domain: "domain";
        flow: "flow";
        step: "step";
        article: "article";
        entity: "entity";
        topic: "topic";
        claim: "claim";
        source: "source";
    }>;
    name: z.ZodString;
    filePath: z.ZodOptional<z.ZodString>;
    lineRange: z.ZodOptional<z.ZodTuple<[z.ZodNumber, z.ZodNumber], null>>;
    summary: z.ZodString;
    tags: z.ZodArray<z.ZodString>;
    complexity: z.ZodEnum<{
        simple: "simple";
        moderate: "moderate";
        complex: "complex";
    }>;
    languageNotes: z.ZodOptional<z.ZodString>;
    domainMeta: z.ZodOptional<z.ZodObject<{
        entities: z.ZodOptional<z.ZodArray<z.ZodString>>;
        businessRules: z.ZodOptional<z.ZodArray<z.ZodString>>;
        crossDomainInteractions: z.ZodOptional<z.ZodArray<z.ZodString>>;
        entryPoint: z.ZodOptional<z.ZodString>;
        entryType: z.ZodOptional<z.ZodEnum<{
            http: "http";
            cli: "cli";
            event: "event";
            cron: "cron";
            manual: "manual";
        }>>;
    }, z.core.$loose>>;
    knowledgeMeta: z.ZodOptional<z.ZodObject<{
        wikilinks: z.ZodOptional<z.ZodArray<z.ZodString>>;
        backlinks: z.ZodOptional<z.ZodArray<z.ZodString>>;
        category: z.ZodOptional<z.ZodString>;
        content: z.ZodOptional<z.ZodString>;
    }, z.core.$loose>>;
}, z.core.$loose>;
export declare const GraphEdgeSchema: z.ZodObject<{
    source: z.ZodString;
    target: z.ZodString;
    type: z.ZodEnum<{
        imports: "imports";
        exports: "exports";
        contains: "contains";
        inherits: "inherits";
        implements: "implements";
        calls: "calls";
        subscribes: "subscribes";
        publishes: "publishes";
        middleware: "middleware";
        reads_from: "reads_from";
        writes_to: "writes_to";
        transforms: "transforms";
        validates: "validates";
        depends_on: "depends_on";
        tested_by: "tested_by";
        configures: "configures";
        related: "related";
        similar_to: "similar_to";
        deploys: "deploys";
        serves: "serves";
        provisions: "provisions";
        triggers: "triggers";
        migrates: "migrates";
        documents: "documents";
        routes: "routes";
        defines_schema: "defines_schema";
        contains_flow: "contains_flow";
        flow_step: "flow_step";
        cross_domain: "cross_domain";
        cites: "cites";
        contradicts: "contradicts";
        builds_on: "builds_on";
        exemplifies: "exemplifies";
        categorized_under: "categorized_under";
        authored_by: "authored_by";
    }>;
    direction: z.ZodEnum<{
        forward: "forward";
        backward: "backward";
        bidirectional: "bidirectional";
    }>;
    description: z.ZodOptional<z.ZodString>;
    weight: z.ZodNumber;
}, z.core.$strip>;
export declare const LayerSchema: z.ZodObject<{
    id: z.ZodString;
    name: z.ZodString;
    description: z.ZodString;
    nodeIds: z.ZodArray<z.ZodString>;
}, z.core.$strip>;
export declare const TourStepSchema: z.ZodObject<{
    order: z.ZodNumber;
    title: z.ZodString;
    description: z.ZodString;
    nodeIds: z.ZodArray<z.ZodString>;
    languageLesson: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const ProjectMetaSchema: z.ZodObject<{
    name: z.ZodString;
    languages: z.ZodArray<z.ZodString>;
    frameworks: z.ZodArray<z.ZodString>;
    description: z.ZodString;
    analyzedAt: z.ZodString;
    gitCommitHash: z.ZodString;
}, z.core.$strip>;
export declare const KnowledgeGraphSchema: z.ZodObject<{
    version: z.ZodString;
    kind: z.ZodOptional<z.ZodEnum<{
        codebase: "codebase";
        knowledge: "knowledge";
    }>>;
    project: z.ZodObject<{
        name: z.ZodString;
        languages: z.ZodArray<z.ZodString>;
        frameworks: z.ZodArray<z.ZodString>;
        description: z.ZodString;
        analyzedAt: z.ZodString;
        gitCommitHash: z.ZodString;
    }, z.core.$strip>;
    nodes: z.ZodArray<z.ZodObject<{
        id: z.ZodString;
        type: z.ZodEnum<{
            function: "function";
            file: "file";
            class: "class";
            module: "module";
            concept: "concept";
            config: "config";
            document: "document";
            service: "service";
            table: "table";
            endpoint: "endpoint";
            pipeline: "pipeline";
            schema: "schema";
            resource: "resource";
            domain: "domain";
            flow: "flow";
            step: "step";
            article: "article";
            entity: "entity";
            topic: "topic";
            claim: "claim";
            source: "source";
        }>;
        name: z.ZodString;
        filePath: z.ZodOptional<z.ZodString>;
        lineRange: z.ZodOptional<z.ZodTuple<[z.ZodNumber, z.ZodNumber], null>>;
        summary: z.ZodString;
        tags: z.ZodArray<z.ZodString>;
        complexity: z.ZodEnum<{
            simple: "simple";
            moderate: "moderate";
            complex: "complex";
        }>;
        languageNotes: z.ZodOptional<z.ZodString>;
        domainMeta: z.ZodOptional<z.ZodObject<{
            entities: z.ZodOptional<z.ZodArray<z.ZodString>>;
            businessRules: z.ZodOptional<z.ZodArray<z.ZodString>>;
            crossDomainInteractions: z.ZodOptional<z.ZodArray<z.ZodString>>;
            entryPoint: z.ZodOptional<z.ZodString>;
            entryType: z.ZodOptional<z.ZodEnum<{
                http: "http";
                cli: "cli";
                event: "event";
                cron: "cron";
                manual: "manual";
            }>>;
        }, z.core.$loose>>;
        knowledgeMeta: z.ZodOptional<z.ZodObject<{
            wikilinks: z.ZodOptional<z.ZodArray<z.ZodString>>;
            backlinks: z.ZodOptional<z.ZodArray<z.ZodString>>;
            category: z.ZodOptional<z.ZodString>;
            content: z.ZodOptional<z.ZodString>;
        }, z.core.$loose>>;
    }, z.core.$loose>>;
    edges: z.ZodArray<z.ZodObject<{
        source: z.ZodString;
        target: z.ZodString;
        type: z.ZodEnum<{
            imports: "imports";
            exports: "exports";
            contains: "contains";
            inherits: "inherits";
            implements: "implements";
            calls: "calls";
            subscribes: "subscribes";
            publishes: "publishes";
            middleware: "middleware";
            reads_from: "reads_from";
            writes_to: "writes_to";
            transforms: "transforms";
            validates: "validates";
            depends_on: "depends_on";
            tested_by: "tested_by";
            configures: "configures";
            related: "related";
            similar_to: "similar_to";
            deploys: "deploys";
            serves: "serves";
            provisions: "provisions";
            triggers: "triggers";
            migrates: "migrates";
            documents: "documents";
            routes: "routes";
            defines_schema: "defines_schema";
            contains_flow: "contains_flow";
            flow_step: "flow_step";
            cross_domain: "cross_domain";
            cites: "cites";
            contradicts: "contradicts";
            builds_on: "builds_on";
            exemplifies: "exemplifies";
            categorized_under: "categorized_under";
            authored_by: "authored_by";
        }>;
        direction: z.ZodEnum<{
            forward: "forward";
            backward: "backward";
            bidirectional: "bidirectional";
        }>;
        description: z.ZodOptional<z.ZodString>;
        weight: z.ZodNumber;
    }, z.core.$strip>>;
    layers: z.ZodArray<z.ZodObject<{
        id: z.ZodString;
        name: z.ZodString;
        description: z.ZodString;
        nodeIds: z.ZodArray<z.ZodString>;
    }, z.core.$strip>>;
    tour: z.ZodArray<z.ZodObject<{
        order: z.ZodNumber;
        title: z.ZodString;
        description: z.ZodString;
        nodeIds: z.ZodArray<z.ZodString>;
        languageLesson: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
}, z.core.$strip>;
export interface GraphIssue {
    level: "auto-corrected" | "dropped" | "fatal";
    category: string;
    message: string;
    path?: string;
}
export interface ValidationResult {
    success: boolean;
    data?: z.infer<typeof KnowledgeGraphSchema>;
    /** @deprecated Use issues/fatal instead */
    errors?: string[];
    issues: GraphIssue[];
    fatal?: string;
}
export declare function normalizeGraph(data: unknown): unknown;
export declare function validateGraph(data: unknown): ValidationResult;
//# sourceMappingURL=schema.d.ts.map