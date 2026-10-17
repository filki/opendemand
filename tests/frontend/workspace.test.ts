import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	isNoWorkspaceError,
	persistLocaleForWorkspace
} from '../../src/lib/frontend/workspace.ts';
import type { Locale } from '../../src/lib/frontend/i18n.ts';

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

test('locale save refreshes only the workspace that initiated the change', async () => {
	let currentPath: string | undefined = '/workspaces/one';
	const events: string[] = [];
	const result = await persistLocaleForWorkspace(
		'/workspaces/one',
		'pl',
		() => currentPath,
		async (locale: Locale) => {
			events.push(`save:${locale}`);
		},
		async () => {
			events.push('refresh');
		}
	);

	assert.equal(result, true, 'same workspace should refresh after a successful save');
	assert.deepEqual(events, ['save:pl', 'refresh'], 'save must complete before the workspace refresh');
});

test('locale save does not refresh a different workspace after an in-flight switch', async () => {
	let currentPath: string | undefined = '/workspaces/one';
	const events: string[] = [];
	const result = await persistLocaleForWorkspace(
		'/workspaces/one',
		'es',
		() => currentPath,
		async (locale: Locale) => {
			events.push(`save:${locale}`);
			currentPath = '/workspaces/two';
		},
		async () => {
			events.push('refresh');
		}
	);

	assert.equal(result, false, 'a stale locale save must not refresh the newly opened workspace');
	assert.deepEqual(events, ['save:es'], 'workspace change during the write must skip invalidation');
});

test('locale save skips persistence if its workspace is no longer active', async () => {
	const events: string[] = [];
	const result = await persistLocaleForWorkspace(
		'/workspaces/one',
		'pl',
		() => '/workspaces/two',
		async () => {
			events.push('save');
		},
		async () => {
			events.push('refresh');
		}
	);

	assert.equal(result, false, 'stale locale event should be ignored');
	assert.deepEqual(events, [], 'an already-switched workspace must not receive the locale write');
});

test('locale save errors propagate without refreshing workspace state', async () => {
	const failure = new Error('locale persistence failed');
	const events: string[] = [];
	await assert.rejects(
		persistLocaleForWorkspace(
			'/workspaces/one',
			'pl',
			() => '/workspaces/one',
			async () => {
				events.push('save');
				throw failure;
			},
			async () => {
				events.push('refresh');
			}
		),
		failure
	);
	assert.deepEqual(events, ['save'], 'failed persistence must not refresh workspace state');
});
