//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.protocol.ecc;

import static org.junit.Assert.*;

import java.util.Arrays;
import org.junit.Test;
import org.signal.libsignal.protocol.InvalidKeyException;

public class Curve25519Test {

  @Test
  public void testAgreement() throws InvalidKeyException {
    byte[] alicePublic = {
      (byte) 0x05, (byte) 0x1b, (byte) 0xb7, (byte) 0x59, (byte) 0x66,
      (byte) 0xf2, (byte) 0xe9, (byte) 0x3a, (byte) 0x36, (byte) 0x91,
      (byte) 0xdf, (byte) 0xff, (byte) 0x94, (byte) 0x2b, (byte) 0xb2,
      (byte) 0xa4, (byte) 0x66, (byte) 0xa1, (byte) 0xc0, (byte) 0x8b,
      (byte) 0x8d, (byte) 0x78, (byte) 0xca, (byte) 0x3f, (byte) 0x4d,
      (byte) 0x6d, (byte) 0xf8, (byte) 0xb8, (byte) 0xbf, (byte) 0xa2,
      (byte) 0xe4, (byte) 0xee, (byte) 0x28
    };

    byte[] alicePrivate = {
      (byte) 0xc8, (byte) 0x06, (byte) 0x43, (byte) 0x9d, (byte) 0xc9,
      (byte) 0xd2, (byte) 0xc4, (byte) 0x76, (byte) 0xff, (byte) 0xed,
      (byte) 0x8f, (byte) 0x25, (byte) 0x80, (byte) 0xc0, (byte) 0x88,
      (byte) 0x8d, (byte) 0x58, (byte) 0xab, (byte) 0x40, (byte) 0x6b,
      (byte) 0xf7, (byte) 0xae, (byte) 0x36, (byte) 0x98, (byte) 0x87,
      (byte) 0x90, (byte) 0x21, (byte) 0xb9, (byte) 0x6b, (byte) 0xb4,
      (byte) 0xbf, (byte) 0x59
    };

    byte[] bobPublic = {
      (byte) 0x05, (byte) 0x65, (byte) 0x36, (byte) 0x14, (byte) 0x99,
      (byte) 0x3d, (byte) 0x2b, (byte) 0x15, (byte) 0xee, (byte) 0x9e,
      (byte) 0x5f, (byte) 0xd3, (byte) 0xd8, (byte) 0x6c, (byte) 0xe7,
      (byte) 0x19, (byte) 0xef, (byte) 0x4e, (byte) 0xc1, (byte) 0xda,
      (byte) 0xae, (byte) 0x18, (byte) 0x86, (byte) 0xa8, (byte) 0x7b,
      (byte) 0x3f, (byte) 0x5f, (byte) 0xa9, (byte) 0x56, (byte) 0x5a,
      (byte) 0x27, (byte) 0xa2, (byte) 0x2f
    };

    byte[] bobPrivate = {
      (byte) 0xb0, (byte) 0x3b, (byte) 0x34, (byte) 0xc3, (byte) 0x3a,
      (byte) 0x1c, (byte) 0x44, (byte) 0xf2, (byte) 0x25, (byte) 0xb6,
      (byte) 0x62, (byte) 0xd2, (byte) 0xbf, (byte) 0x48, (byte) 0x59,
      (byte) 0xb8, (byte) 0x13, (byte) 0x54, (byte) 0x11, (byte) 0xfa,
      (byte) 0x7b, (byte) 0x03, (byte) 0x86, (byte) 0xd4, (byte) 0x5f,
      (byte) 0xb7, (byte) 0x5d, (byte) 0xc5, (byte) 0xb9, (byte) 0x1b,
      (byte) 0x44, (byte) 0x66
    };

    byte[] shared = {
      (byte) 0x32, (byte) 0x5f, (byte) 0x23, (byte) 0x93, (byte) 0x28,
      (byte) 0x94, (byte) 0x1c, (byte) 0xed, (byte) 0x6e, (byte) 0x67,
      (byte) 0x3b, (byte) 0x86, (byte) 0xba, (byte) 0x41, (byte) 0x01,
      (byte) 0x74, (byte) 0x48, (byte) 0xe9, (byte) 0x9b, (byte) 0x64,
      (byte) 0x9a, (byte) 0x9c, (byte) 0x38, (byte) 0x06, (byte) 0xc1,
      (byte) 0xdd, (byte) 0x7c, (byte) 0xa4, (byte) 0xc4, (byte) 0x77,
      (byte) 0xe6, (byte) 0x29
    };

    ECPublicKey alicePublicKey = new ECPublicKey(alicePublic, 0);
    ECPrivateKey alicePrivateKey = new ECPrivateKey(alicePrivate);

    ECPublicKey bobPublicKey = new ECPublicKey(bobPublic, 0);
    ECPrivateKey bobPrivateKey = new ECPrivateKey(bobPrivate);

    byte[] sharedOne = bobPrivateKey.calculateAgreement(alicePublicKey);
    byte[] sharedTwo = alicePrivateKey.calculateAgreement(bobPublicKey);

    assertTrue(Arrays.equals(sharedOne, shared));
    assertTrue(Arrays.equals(sharedTwo, shared));
  }

  @Test
  public void testRandomAgreements() throws InvalidKeyException {
    for (int i = 0; i < 50; i++) {
      ECKeyPair alice = ECKeyPair.generate();
      ECKeyPair bob = ECKeyPair.generate();

      byte[] sharedAlice = alice.getPrivateKey().calculateAgreement(bob.getPublicKey());
      byte[] sharedBob = bob.getPrivateKey().calculateAgreement(alice.getPublicKey());

      assertTrue(Arrays.equals(sharedAlice, sharedBob));
    }
  }

  @Test
  public void testSignature() throws InvalidKeyException {
    byte[] aliceIdentityPrivate = {
      (byte) 0xc0, (byte) 0x97, (byte) 0x24, (byte) 0x84, (byte) 0x12,
      (byte) 0xe5, (byte) 0x8b, (byte) 0xf0, (byte) 0x5d, (byte) 0xf4,
      (byte) 0x87, (byte) 0x96, (byte) 0x82, (byte) 0x05, (byte) 0x13,
      (byte) 0x27, (byte) 0x94, (byte) 0x17, (byte) 0x8e, (byte) 0x36,
      (byte) 0x76, (byte) 0x37, (byte) 0xf5, (byte) 0x81, (byte) 0x8f,
      (byte) 0x81, (byte) 0xe0, (byte) 0xe6, (byte) 0xce, (byte) 0x73,
      (byte) 0xe8, (byte) 0x65
    };

    byte[] aliceIdentityPublic = {
      (byte) 0x05, (byte) 0xab, (byte) 0x7e, (byte) 0x71, (byte) 0x7d,
      (byte) 0x4a, (byte) 0x16, (byte) 0x3b, (byte) 0x7d, (byte) 0x9a,
      (byte) 0x1d, (byte) 0x80, (byte) 0x71, (byte) 0xdf, (byte) 0xe9,
      (byte) 0xdc, (byte) 0xf8, (byte) 0xcd, (byte) 0xcd, (byte) 0x1c,
      (byte) 0xea, (byte) 0x33, (byte) 0x39, (byte) 0xb6, (byte) 0x35,
      (byte) 0x6b, (byte) 0xe8, (byte) 0x4d, (byte) 0x88, (byte) 0x7e,
      (byte) 0x32, (byte) 0x2c, (byte) 0x64
    };

    byte[] aliceEphemeralPublic = {
      (byte) 0x05, (byte) 0xed, (byte) 0xce, (byte) 0x9d, (byte) 0x9c,
      (byte) 0x41, (byte) 0x5c, (byte) 0xa7, (byte) 0x8c, (byte) 0xb7,
      (byte) 0x25, (byte) 0x2e, (byte) 0x72, (byte) 0xc2, (byte) 0xc4,
      (byte) 0xa5, (byte) 0x54, (byte) 0xd3, (byte) 0xeb, (byte) 0x29,
      (byte) 0x48, (byte) 0x5a, (byte) 0x0e, (byte) 0x1d, (byte) 0x50,
      (byte) 0x31, (byte) 0x18, (byte) 0xd1, (byte) 0xa8, (byte) 0x2d,
      (byte) 0x99, (byte) 0xfb, (byte) 0x4a
    };

    byte[] aliceSignature = {
      (byte) 0x5d, (byte) 0xe8, (byte) 0x8c, (byte) 0xa9, (byte) 0xa8,
      (byte) 0x9b, (byte) 0x4a, (byte) 0x11, (byte) 0x5d, (byte) 0xa7,
      (byte) 0x91, (byte) 0x09, (byte) 0xc6, (byte) 0x7c, (byte) 0x9c,
      (byte) 0x74, (byte) 0x64, (byte) 0xa3, (byte) 0xe4, (byte) 0x18,
      (byte) 0x02, (byte) 0x74, (byte) 0xf1, (byte) 0xcb, (byte) 0x8c,
      (byte) 0x63, (byte) 0xc2, (byte) 0x98, (byte) 0x4e, (byte) 0x28,
      (byte) 0x6d, (byte) 0xfb, (byte) 0xed, (byte) 0xe8, (byte) 0x2d,
      (byte) 0xeb, (byte) 0x9d, (byte) 0xcd, (byte) 0x9f, (byte) 0xae,
      (byte) 0x0b, (byte) 0xfb, (byte) 0xb8, (byte) 0x21, (byte) 0x56,
      (byte) 0x9b, (byte) 0x3d, (byte) 0x90, (byte) 0x01, (byte) 0xbd,
      (byte) 0x81, (byte) 0x30, (byte) 0xcd, (byte) 0x11, (byte) 0xd4,
      (byte) 0x86, (byte) 0xce, (byte) 0xf0, (byte) 0x47, (byte) 0xbd,
      (byte) 0x60, (byte) 0xb8, (byte) 0x6e, (byte) 0x88
    };

    ECPrivateKey alicePrivateKey = new ECPrivateKey(aliceIdentityPrivate);
    ECPublicKey alicePublicKey = new ECPublicKey(aliceIdentityPublic, 0);
    ECPublicKey aliceEphemeral = new ECPublicKey(aliceEphemeralPublic, 0);

    if (!alicePublicKey.verifySignature(aliceEphemeral.serialize(), aliceSignature)) {
      throw new AssertionError("Sig verification failed!");
    }

    byte[] aliceKey = aliceEphemeral.getPublicKeyBytes();
    assertTrue(aliceKey.length == 32);

    for (int i = 0; i < aliceSignature.length; i++) {
      byte[] modifiedSignature = new byte[aliceSignature.length];
      System.arraycopy(aliceSignature, 0, modifiedSignature, 0, modifiedSignature.length);

      modifiedSignature[i] ^= 0x01;

      if (alicePublicKey.verifySignature(aliceEphemeral.serialize(), modifiedSignature)) {
        throw new AssertionError("Sig verification succeeded!");
      }
    }
  }

  @Test
  public void testDecodeSize() throws InvalidKeyException {
    ECKeyPair keyPair = ECKeyPair.generate();
    byte[] serializedPublic = keyPair.getPublicKey().serialize();
    assertEquals(serializedPublic.length, ECPublicKey.KEY_SIZE);

    ECPublicKey justRight = new ECPublicKey(serializedPublic, 0);

    assertThrows(
        "too small w/ offset",
        InvalidKeyException.class,
        () -> new ECPublicKey(serializedPublic, 1));

    byte[] truncated = new byte[31];
    System.arraycopy(serializedPublic, 1, truncated, 0, truncated.length);
    assertThrows("too small", InvalidKeyException.class, () -> new ECPublicKey(truncated, 0));
    assertThrows("too small", InvalidKeyException.class, () -> new ECPrivateKey(truncated));
    assertThrows(
        "too small", InvalidKeyException.class, () -> ECPublicKey.fromPublicKeyBytes(truncated));

    assertThrows("empty", InvalidKeyException.class, () -> new ECPublicKey(new byte[0], 0));
    assertThrows("empty", InvalidKeyException.class, () -> new ECPrivateKey(new byte[0]));
    assertThrows(
        "empty", InvalidKeyException.class, () -> ECPublicKey.fromPublicKeyBytes(new byte[0]));

    byte[] badKeyType = new byte[33];
    System.arraycopy(serializedPublic, 0, badKeyType, 0, serializedPublic.length);
    badKeyType[0] = 0x01;
    assertThrows(InvalidKeyException.class, () -> new ECPublicKey(badKeyType, 0));

    // We allow extra trailing space for keys with type bytes for historical compatibility.
    byte[] extraSpace = new byte[serializedPublic.length + 1];
    System.arraycopy(serializedPublic, 0, extraSpace, 0, serializedPublic.length);
    ECPublicKey extra = new ECPublicKey(extraSpace, 0);
    assertThrows("too big", InvalidKeyException.class, () -> new ECPrivateKey(extraSpace));
    assertThrows(
        "too big", InvalidKeyException.class, () -> ECPublicKey.fromPublicKeyBytes(extraSpace));

    byte[] offsetSpace = new byte[serializedPublic.length + 1];
    System.arraycopy(serializedPublic, 0, offsetSpace, 1, serializedPublic.length);
    ECPublicKey offset = new ECPublicKey(offsetSpace, 1);

    assertTrue(Arrays.equals(serializedPublic, justRight.serialize()));
    assertTrue(Arrays.equals(extra.serialize(), serializedPublic));
    assertTrue(Arrays.equals(offset.serialize(), serializedPublic));
  }
}
