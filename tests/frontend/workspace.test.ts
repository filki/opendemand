import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isNoWorkspaceError } from '../../src/lib/frontend/workspace.ts';

test('no_workspace errors count as an empty workspace state', () => {
	assert.equal(
		isNoWorkspaceError({ code: 'no_workspace', message: 'Create or open a workspace first.' }),
		true
	);
});

test('other failures stay errors', () => {
	assert.equal(isNoWorkspaceError({ code: 'io_error', message: 'boom' }), false);
	assert.equal(isNoWorkspaceError(new Error('boom')), false);
	assert.equal(isNoWorkspaceError('no_workspace'), false);
	assert.equal(isNoWorkspaceError(null), false);
	assert.equal(isNoWorkspaceError(undefined), false);
});
