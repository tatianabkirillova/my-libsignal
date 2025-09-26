//
// Copyright 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

import { assert } from 'chai';
import * as MessageBackup from '../MessageBackup.js';
import * as util from './util.js';
import { Aci } from '../Address.js';
import { Uint8ArrayInputStream, ErrorInputStream } from './ioutil.js';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { hkdf, LogLevel } from '../index.js';
import {
  AccountEntropyPool,
  BackupForwardSecrecyToken,
  BackupKey,
} from '../AccountKeys.js';
import { Readable } from 'node:stream';
import { InputStream } from '../io.js';
import { assertArrayNotEquals } from './util.js';

util.initLogger(LogLevel.Trace);

describe('AccountEntropyPool', () => {
  describe('isValid', () => {
    assert.isFalse(AccountEntropyPool.isValid('invalid key'));
    assert.isTrue(
      AccountEntropyPool.isValid(
        '0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqr'
      )
    );
  });
});

describe('MessageBackup', () => {
  const accountEntropy = 'm'.repeat(64);
  const aci = Aci.fromUuidBytes(new Uint8Array(16).fill(0x11));
  const testKey = new MessageBackup.MessageBackupKey({ accountEntropy, aci });
  const purpose = MessageBackup.Purpose.RemoteBackup;

  describe('MessageBackupKey', () => {
    it('provides its HMAC and AES keys', () => {
      // Just check some basic expectations.
      assert.equal(32, testKey.hmacKey.length);
      assert.equal(32, testKey.aesKey.length);
      assertArrayNotEquals(testKey.hmacKey, testKey.aesKey);
    });

    it('can derive from a forward secrecy token', () => {
      const forwardSecrecyToken = new BackupForwardSecrecyToken(
        new Uint8Array(32).fill(0xbf)
      );
      const keyFromAep = new MessageBackup.MessageBackupKey({
        accountEntropy,
        aci,
        forwardSecrecyToken,
      });
      assertArrayNotEquals(keyFromAep.aesKey, testKey.aesKey);

      const backupKey = new BackupKey(new Uint8Array(32).fill(0xba));
      const backupId = new Uint8Array(16).fill(0x1d);
      const keyFromBackupInfo = new MessageBackup.MessageBackupKey({
        backupKey,
        backupId,
        forwardSecrecyToken,
      });
      assertArrayNotEquals(
        keyFromBackupInfo.aesKey,
        new MessageBackup.MessageBackupKey({ backupKey, backupId }).aesKey
      );
    });
  });

  describe('validate', () => {
    it('successfully validates a minimal backup', async () => {
      const input = fs.readFileSync(
        path.join(
          import.meta.dirname,
          '../../ts/test/new_account.binproto.encrypted'
        )
      );

      const outcome = await MessageBackup.validate(
        testKey,
        purpose,
        () => new Uint8ArrayInputStream(input),
        BigInt(input.length)
      );
      assert.equal(outcome.errorMessage, null);

      // If we manually derive the test key's backup key and ID, we should get the same outcome.
      const backupKey = hkdf(
        32,
        Buffer.from(accountEntropy, 'utf8'),
        Buffer.from('20240801_SIGNAL_BACKUP_KEY', 'utf8'),
        null
      );
      const backupId = hkdf(
        16,
        backupKey,
        Buffer.concat([
          Buffer.from('20241024_SIGNAL_BACKUP_ID:', 'utf8'),
          aci.getServiceIdBinary(),
        ]),
        null
      );
      const testKeyFromBackupId = new MessageBackup.MessageBackupKey({
        backupKey: new BackupKey(backupKey),
        backupId,
      });

      const outcome2 = await MessageBackup.validate(
        testKeyFromBackupId,
        purpose,
        () => new Uint8ArrayInputStream(input),
        BigInt(input.length)
      );
      assert.equal(outcome2.errorMessage, null);
    });

    it('throws on empty input', async () => {
      try {
        await MessageBackup.validate(
          testKey,
          purpose,
          () => new Uint8ArrayInputStream(new Uint8Array()),
          0n
        );
        assert.fail('did not throw');
      } catch (e) {
        assert.instanceOf(e, Error);
        assert.equal(e.message, 'unexpected end of file');
      }
    });

    it('throws a raised IO error', async () => {
      try {
        await MessageBackup.validate(
          testKey,
          purpose,
          () => new ErrorInputStream(),
          234n
        );
        assert.fail('did not throw');
      } catch (e) {
        assert.instanceOf(e, ErrorInputStream.Error);
      }
    });

    it('closes the streams it creates', async () => {
      let openCount = 0;
      let closeCount = 0;
      class CloseCountingInputStream extends InputStream {
        /* eslint-disable @typescript-eslint/require-await */
        async close(): Promise<void> {
          closeCount += 1;
        }
        async read(_amount: number): Promise<Uint8Array> {
          return Uint8Array.of();
        }
        async skip(amount: number): Promise<void> {
          if (amount > 0) {
            throw Error("can't skip in an empty stream");
          }
        }
        /* eslint-enable @typescript-eslint/require-await */
      }

      await assert.isRejected(
        MessageBackup.validate(
          testKey,
          purpose,
          () => {
            openCount += 1;
            return new CloseCountingInputStream();
          },
          0n
        )
      );
      assert.isAbove(openCount, 0, 'never opened?');
      assert.equal(openCount, closeCount, 'failed to close all streams');
    });
  });
});

const exampleBackup = fs.readFileSync(
  path.join(import.meta.dirname, '../../ts/test/canonical-backup.binproto')
);

describe('ComparableBackup', () => {
  describe('exampleBackup', () => {
    it('stringifies to the expected value', async () => {
      const comparable = await MessageBackup.ComparableBackup.fromUnencrypted(
        MessageBackup.Purpose.RemoteBackup,
        new Uint8ArrayInputStream(exampleBackup),
        BigInt(exampleBackup.length)
      );

      const expectedOutput = fs.readFileSync(
        path.join(
          import.meta.dirname,
          '../../ts/test/canonical-backup.expected.json'
        )
      );
      const output = comparable.comparableString();
      assert.equal(output, new String(expectedOutput));
    });
  });
});

describe('OnlineBackupValidator', () => {
  it('can read frames from a valid file', () => {
    // `Readable.read` normally returns `any`, because it supports settable encodings.
    // Here we override that `read` member with one that always produces a Uint8Array,
    // for more convenient use in the test. Note that this is unchecked.
    type ReadableUsingUint8Array = Omit<Readable, 'read'> & {
      read: (size: number) => Uint8Array;
    };
    const input: ReadableUsingUint8Array = new Readable();
    input.push(exampleBackup);
    input.push(null);

    const backupInfoLength = input.read(1)[0];
    assert.isBelow(backupInfoLength, 0x80, 'single-byte varint');
    const backupInfo = input.read(backupInfoLength);
    assert.equal(backupInfo.length, backupInfoLength, 'unexpected EOF');
    const backup = new MessageBackup.OnlineBackupValidator(
      backupInfo,
      MessageBackup.Purpose.RemoteBackup
    );

    let frameLengthBuf;
    while ((frameLengthBuf = input.read(1))) {
      let frameLength = frameLengthBuf[0];
      // Tiny varint parser, only supports two bytes.
      if (frameLength >= 0x80) {
        const secondByte = input.read(1)[0];
        assert.isBelow(secondByte, 0x80, 'at most a two-byte varint');
        frameLength -= 0x80;
        frameLength |= secondByte << 7;
      }
      const frame = input.read(frameLength);
      assert.equal(frame.length, frameLength, 'unexpected EOF');
      backup.addFrame(frame);
    }

    backup.finalize();
  });

  it('rejects invalid BackupInfo', () => {
    assert.throws(
      () =>
        new MessageBackup.OnlineBackupValidator(
          Uint8Array.of(),
          MessageBackup.Purpose.RemoteBackup
        )
    );
  });

  // The following payload was generated via protoscope.
  // % protoscope -s | base64
  // The fields are described by Backup.proto.
  //
  // 1: 1
  // 2: 1731715200000
  // 3: {`00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff`}
  const VALID_BACKUP_INFO: Buffer = Buffer.from(
    'CAEQgOiTkrMyGiAAESIzRFVmd4iZqrvM3e7/ABEiM0RVZneImaq7zN3u/w==',
    'base64'
  );

  it('rejects invalid Frames', () => {
    const backup = new MessageBackup.OnlineBackupValidator(
      VALID_BACKUP_INFO,
      MessageBackup.Purpose.RemoteBackup
    );
    assert.throws(() => backup.addFrame(Uint8Array.of()));
  });

  it('rejects invalid backups on finalize', () => {
    const backup = new MessageBackup.OnlineBackupValidator(
      VALID_BACKUP_INFO,
      MessageBackup.Purpose.RemoteBackup
    );
    assert.throws(() => backup.finalize());
  });
});
