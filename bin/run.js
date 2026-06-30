#!/usr/bin/env node
'use strict';
// CLI entry point. Compiled sources live in ../build.
require('../build/index.js').buildProgram().parseAsync(process.argv);
