import test from 'node:test';
import assert from 'node:assert/strict';
import {readFixturePort} from './browser-fixture.mjs';

test('port publication tolerates missing and temporarily locked files', () => {
  for (const code of ['ENOENT','EBUSY','EPERM']) {
    assert.equal(readFixturePort('fixture',()=>{throw Object.assign(new Error(code),{code});}),null);
  }
  assert.equal(readFixturePort('fixture',()=> '54321\n/devtools/browser/fixture\n'),54321);
});
test('partial or invalid port publication is not a usable endpoint', () => {
  for (const value of ['', '543', '0\n', '65536\n', '123x\n']) {
    assert.equal(readFixturePort('fixture',()=>value),null);
  }
});
test('unexpected read failures retain the original error', () => {
  const error=Object.assign(new Error('disk failure'),{code:'EIO'});
  assert.throws(()=>readFixturePort('fixture',()=>{throw error;}),e=>e===error);
});
