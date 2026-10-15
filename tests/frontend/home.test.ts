import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	canManageWorkspace,
	confirmWorkspaceName,
	createWorkspaceHint,
	isWorkspaceNameValid,
	isWorkspaceOpenable,
	isWorkspaceRemovable,
	workspaceActionHint
} from '../../src/lib/frontend/home.ts';

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
		exists: true,
		is_default: false
	};
	assert.equal(isWorkspaceOpenable(entry), true);
	assert.equal(isWorkspaceOpenable({ ...entry, exists: false }), false);
});

test('the default workspace entry is never removable', () => {
	const entry = {
		path: '/data/one',
		name: 'one',
		last_opened_at: '2026-01-01T00:00:00Z',
		exists: true,
		is_default: false
	};
	assert.equal(isWorkspaceRemovable(entry), true);
	assert.equal(isWorkspaceRemovable({ ...entry, is_default: true }), false);
});

test('only existing non-default workspaces can be renamed or deleted', () => {
	const entry = {
		path: '/data/one',
		name: 'one',
		last_opened_at: '2026-01-01T00:00:00Z',
		exists: true,
		is_default: false
	};
	assert.equal(canManageWorkspace(entry), true);
	assert.equal(canManageWorkspace({ ...entry, is_default: true }), false);
	assert.equal(canManageWorkspace({ ...entry, exists: false }), false);
});

test('the typed confirmation compares the name exactly', () => {
	assert.equal(confirmWorkspaceName('planner 2026', 'planner 2026'), true);
	assert.equal(confirmWorkspaceName(' planner 2026', 'planner 2026'), false);
	assert.equal(confirmWorkspaceName('Planner 2026', 'planner 2026'), false);
	assert.equal(confirmWorkspaceName('', 'planner 2026'), false);
});

test('workspace names are validated as single folder names', () => {
	assert.equal(isWorkspaceNameValid('planner 2026'), true);
	assert.equal(isWorkspaceNameValid('  trimmed  '), true);
	assert.equal(isWorkspaceNameValid(''), false);
	assert.equal(isWorkspaceNameValid('   '), false);
	assert.equal(isWorkspaceNameValid('.'), false);
	assert.equal(isWorkspaceNameValid('..'), false);
	assert.equal(isWorkspaceNameValid('a/b'), false);
	assert.equal(isWorkspaceNameValid('a\\b'), false);
	assert.equal(isWorkspaceNameValid('x'.repeat(129)), false);
});

test('workspace action failures get a targeted hint', () => {
	assert.match(String(workspaceActionHint('invalid_workspace_name')), /single folder name/);
	assert.match(String(workspaceActionHint('workspace_name_taken')), /already exists/);
	assert.match(String(workspaceActionHint('confirm_name_mismatch')), /exactly/);
	assert.match(String(workspaceActionHint('workspace_default')), /default workspace/);
	assert.equal(workspaceActionHint('io_error'), null);
});
