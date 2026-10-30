#!/usr/bin/env node
// gs2d MCP server (stdio). Registered in the repo's .mcp.json; see README.md / SKILL.md.
import { serve } from './lib.mjs';

serve(process.stdin, process.stdout);
process.stdin.on('end', () => process.exit(0));
