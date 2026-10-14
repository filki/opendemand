import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createWorkspaceHint, isWorkspaceOpenable } from '../../src/lib/frontend/home.ts';

test('create failures get a targeted hint', () => {
	assert.match(String(createWorkspaceHint('workspace_exists')), /already lives/);
	assert.match(String(createWorkspaceHint('workspace_not_empty')), /empty folder/);
	assert.match(String(createWorkspaceHint('workspace_not_directory')), /folder, not a file/);
	assert.equal(createWorkspaceHint('io_error'), null);
});

test('only entries whose database exists are openable', () => {
	const entry = {
		path: '/data/one',
		name: 'one',
		last_opened_at: '2026-01-01T00:00:00Z',
		exists: true
	};
	assert.equal(isWorkspaceOpenable(entry), true);
	assert.equal(isWorkspaceOpenable({ ...entry, exists: false }), false);
});
