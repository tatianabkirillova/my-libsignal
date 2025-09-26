//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.protocol;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;
import static org.signal.libsignal.internal.FilterExceptions.filterExceptions;
import static org.signal.libsignal.protocol.SessionRecordTest.getAliceBaseKey;

import java.time.Instant;
import java.time.temporal.ChronoUnit;
import java.util.Arrays;
import java.util.Collection;
import java.util.HashSet;
import java.util.Random;
import java.util.Set;
import org.junit.Test;
import org.junit.experimental.runners.Enclosed;
import org.junit.runner.RunWith;
import org.junit.runners.Parameterized;
import org.junit.runners.Parameterized.Parameters;
import org.signal.libsignal.protocol.ecc.ECKeyPair;
import org.signal.libsignal.protocol.kem.KEMKeyPair;
import org.signal.libsignal.protocol.kem.KEMKeyType;
import org.signal.libsignal.protocol.message.CiphertextMessage;
import org.signal.libsignal.protocol.message.PreKeySignalMessage;
import org.signal.libsignal.protocol.message.SignalMessage;
import org.signal.libsignal.protocol.state.IdentityKeyStore;
import org.signal.libsignal.protocol.state.PreKeyBundle;
import org.signal.libsignal.protocol.state.SessionRecord;
import org.signal.libsignal.protocol.state.SignalProtocolStore;
import org.signal.libsignal.protocol.util.Medium;
import org.signal.libsignal.protocol.util.Pair;

@RunWith(Enclosed.class)
public class SessionBuilderTest {
  static final SignalProtocolAddress ALICE_ADDRESS =
      filterExceptions(() -> new SignalProtocolAddress("+14151111111", 1));
  static final SignalProtocolAddress BOB_ADDRESS =
      filterExceptions(() -> new SignalProtocolAddress("+14152222222", 1));
  static final SignalProtocolAddress MALLORY_ADDRESS =
      filterExceptions(() -> new SignalProtocolAddress("+14153333333", 1));

  @RunWith(Parameterized.class)
  public static class Versioned {
    private final BundleFactory bundleFactory;
    private int expectedVersion;

    public Versioned(BundleFactory bundleFactory, int expectedVersion) {
      this.bundleFactory = bundleFactory;
      this.expectedVersion = expectedVersion;
    }

    @Parameters(name = "v{1}")
    public static Collection<Object[]> data() throws Exception {
      return Arrays.asList(new Object[][] {{new PQXDHBundleFactory(), 4}});
    }

    Pair<SignalProtocolStore, SignalProtocolStore> initializeSessions() {
      try {
        SignalProtocolStore aliceStore = new TestInMemorySignalProtocolStore();
        SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

        SignalProtocolStore bobStore = new TestInMemorySignalProtocolStore();

        PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);

        aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);

        assertTrue(aliceStore.containsSession(BOB_ADDRESS));
        assertTrue(aliceStore.loadSession(BOB_ADDRESS).getSessionVersion() == expectedVersion);

        String originalMessage = "initial hello!";
        SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
        CiphertextMessage outgoingMessage = aliceSessionCipher.encrypt(originalMessage.getBytes());

        assertTrue(outgoingMessage.getType() == CiphertextMessage.PREKEY_TYPE);

        PreKeySignalMessage incomingMessage = new PreKeySignalMessage(outgoingMessage.serialize());

        SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);
        byte[] plaintext = bobSessionCipher.decrypt(incomingMessage, UsePqRatchet.YES);

        assertTrue(bobStore.containsSession(ALICE_ADDRESS));
        assertEquals(bobStore.loadSession(ALICE_ADDRESS).getSessionVersion(), expectedVersion);
        assertNotNull(getAliceBaseKey(bobStore.loadSession(ALICE_ADDRESS)));
        assertTrue(originalMessage.equals(new String(plaintext)));

        CiphertextMessage bobOutgoingMessage = bobSessionCipher.encrypt(originalMessage.getBytes());
        assertTrue(bobOutgoingMessage.getType() == CiphertextMessage.WHISPER_TYPE);

        byte[] alicePlaintext =
            aliceSessionCipher.decrypt(new SignalMessage(bobOutgoingMessage.serialize()));
        assertTrue(new String(alicePlaintext).equals(originalMessage));

        return new Pair<>(aliceStore, bobStore);

      } catch (DuplicateMessageException
          | InvalidKeyException
          | InvalidKeyIdException
          | InvalidMessageException
          | InvalidVersionException
          | LegacyMessageException
          | NoSessionException
          | UntrustedIdentityException e) {
        throw new AssertionError("basic initialization should not encounter any exceptions", e);
      }
    }

    @Test
    public void testBasicPreKey()
        throws InvalidKeyException,
            InvalidVersionException,
            InvalidMessageException,
            InvalidKeyIdException,
            DuplicateMessageException,
            LegacyMessageException,
            UntrustedIdentityException,
            NoSessionException {
      var stores = initializeSessions();
      SignalProtocolStore aliceStore = stores.first();
      SignalProtocolStore bobStore = stores.second();

      runInteraction(aliceStore, bobStore);

      aliceStore = new TestInMemorySignalProtocolStore();
      var aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);
      var aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);

      PreKeyBundle anotherBundle = bundleFactory.createBundle(bobStore);
      aliceSessionBuilder.process(anotherBundle, UsePqRatchet.YES);

      String originalMessage = "Good, fast, cheap: pick two";
      var outgoingMessage = aliceSessionCipher.encrypt(originalMessage.getBytes());

      var bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);
      try {
        bobSessionCipher.decrypt(
            new PreKeySignalMessage(outgoingMessage.serialize()), UsePqRatchet.YES);
        fail("shouldn't be trusted!");
      } catch (UntrustedIdentityException uie) {
        bobStore.saveIdentity(
            ALICE_ADDRESS, new PreKeySignalMessage(outgoingMessage.serialize()).getIdentityKey());
      }

      var plaintext =
          bobSessionCipher.decrypt(
              new PreKeySignalMessage(outgoingMessage.serialize()), UsePqRatchet.YES);
      assertTrue(new String(plaintext).equals(originalMessage));

      Random random = new Random();
      PreKeyBundle badIdentityBundle =
          new PreKeyBundle(
              bobStore.getLocalRegistrationId(),
              1,
              random.nextInt(Medium.MAX_VALUE),
              ECKeyPair.generate().getPublicKey(),
              random.nextInt(Medium.MAX_VALUE),
              anotherBundle.getSignedPreKey(),
              anotherBundle.getSignedPreKeySignature(),
              aliceStore.getIdentityKeyPair().getPublicKey(),
              random.nextInt(Medium.MAX_VALUE),
              anotherBundle.getKyberPreKey(),
              anotherBundle.getKyberPreKeySignature());

      try {
        aliceSessionBuilder.process(badIdentityBundle, UsePqRatchet.YES);
        fail("shoulnd't be trusted!");
      } catch (UntrustedIdentityException uie) {
        // good
      }
    }

    @Test
    public void testRepeatBundleMessage()
        throws InvalidKeyException,
            UntrustedIdentityException,
            InvalidVersionException,
            InvalidMessageException,
            InvalidKeyIdException,
            DuplicateMessageException,
            LegacyMessageException,
            NoSessionException {
      SignalProtocolStore aliceStore = new TestInMemorySignalProtocolStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      SignalProtocolStore bobStore = new TestInMemorySignalProtocolStore();

      PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);
      aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);

      String originalMessage = "Good, fast, cheap: pick two";
      SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
      CiphertextMessage outgoingMessageOne = aliceSessionCipher.encrypt(originalMessage.getBytes());
      CiphertextMessage outgoingMessageTwo = aliceSessionCipher.encrypt(originalMessage.getBytes());

      assertTrue(outgoingMessageOne.getType() == CiphertextMessage.PREKEY_TYPE);
      assertTrue(outgoingMessageTwo.getType() == CiphertextMessage.PREKEY_TYPE);

      PreKeySignalMessage incomingMessage = new PreKeySignalMessage(outgoingMessageOne.serialize());

      SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);

      byte[] plaintext = bobSessionCipher.decrypt(incomingMessage, UsePqRatchet.YES);
      assertTrue(originalMessage.equals(new String(plaintext)));

      CiphertextMessage bobOutgoingMessage = bobSessionCipher.encrypt(originalMessage.getBytes());

      byte[] alicePlaintext =
          aliceSessionCipher.decrypt(new SignalMessage(bobOutgoingMessage.serialize()));
      assertTrue(originalMessage.equals(new String(alicePlaintext)));

      // The test

      PreKeySignalMessage incomingMessageTwo =
          new PreKeySignalMessage(outgoingMessageTwo.serialize());

      plaintext =
          bobSessionCipher.decrypt(
              new PreKeySignalMessage(incomingMessageTwo.serialize()), UsePqRatchet.YES);
      assertTrue(originalMessage.equals(new String(plaintext)));

      bobOutgoingMessage = bobSessionCipher.encrypt(originalMessage.getBytes());
      alicePlaintext =
          aliceSessionCipher.decrypt(new SignalMessage(bobOutgoingMessage.serialize()));
      assertTrue(originalMessage.equals(new String(alicePlaintext)));
    }

    @Test
    public void testOptionalOneTimePreKey() throws Exception {
      SignalProtocolStore aliceStore = new TestInMemorySignalProtocolStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      SignalProtocolStore bobStore = new TestInMemorySignalProtocolStore();
      PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);

      // Simply remove the pre-key information from a valid bundle
      bobPreKey =
          new PreKeyBundle(
              bobPreKey.getRegistrationId(),
              1,
              -1,
              null,
              bobPreKey.getSignedPreKeyId(),
              bobPreKey.getSignedPreKey(),
              bobPreKey.getSignedPreKeySignature(),
              bobPreKey.getIdentityKey(),
              bobPreKey.getKyberPreKeyId(),
              bobPreKey.getKyberPreKey(),
              bobPreKey.getKyberPreKeySignature());

      aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);

      assertTrue(aliceStore.containsSession(BOB_ADDRESS));
      assertTrue(aliceStore.loadSession(BOB_ADDRESS).getSessionVersion() == expectedVersion);

      String originalMessage = "Good, fast, cheap: pick two";
      SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
      CiphertextMessage outgoingMessage = aliceSessionCipher.encrypt(originalMessage.getBytes());

      assertTrue(outgoingMessage.getType() == CiphertextMessage.PREKEY_TYPE);

      PreKeySignalMessage incomingMessage = new PreKeySignalMessage(outgoingMessage.serialize());
      assertTrue(!incomingMessage.getPreKeyId().isPresent());

      SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);
      byte[] plaintext = bobSessionCipher.decrypt(incomingMessage, UsePqRatchet.YES);

      assertTrue(bobStore.containsSession(ALICE_ADDRESS));
      assertEquals(bobStore.loadSession(ALICE_ADDRESS).getSessionVersion(), expectedVersion);
      assertNotNull(getAliceBaseKey(bobStore.loadSession(ALICE_ADDRESS)));
      assertEquals(originalMessage, new String(plaintext));
    }

    @Test
    public void testExpiresUnacknowledgedSessions()
        throws InvalidKeyException,
            InvalidVersionException,
            InvalidMessageException,
            InvalidKeyIdException,
            DuplicateMessageException,
            LegacyMessageException,
            UntrustedIdentityException,
            NoSessionException {
      SignalProtocolStore aliceStore = new TestInMemorySignalProtocolStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      final SignalProtocolStore bobStore = new TestInMemorySignalProtocolStore();

      PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);

      aliceSessionBuilder.process(bobPreKey, Instant.EPOCH, UsePqRatchet.YES);

      SessionRecord initialSession = aliceStore.loadSession(BOB_ADDRESS);
      assertTrue(initialSession.hasSenderChain(Instant.EPOCH));
      assertFalse(initialSession.hasSenderChain(Instant.EPOCH.plus(90, ChronoUnit.DAYS)));

      String originalMessage = "Good, fast, cheap: pick two";
      SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
      CiphertextMessage outgoingMessage =
          aliceSessionCipher.encrypt(originalMessage.getBytes(), Instant.EPOCH);

      assertTrue(outgoingMessage.getType() == CiphertextMessage.PREKEY_TYPE);

      SessionRecord updatedSession = aliceStore.loadSession(BOB_ADDRESS);
      assertTrue(updatedSession.hasSenderChain(Instant.EPOCH));
      assertFalse(updatedSession.hasSenderChain(Instant.EPOCH.plus(90, ChronoUnit.DAYS)));

      try {
        aliceSessionCipher.encrypt(
            originalMessage.getBytes(), Instant.EPOCH.plus(90, ChronoUnit.DAYS));
        fail("should have expired");
      } catch (NoSessionException e) {
        // Expected
      }
    }

    @Test
    public void testRejectsPreKeyMesageSentFromDifferentUser() throws Exception {
      SignalProtocolStore aliceStore = new TestInMemorySignalProtocolStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      SignalProtocolStore bobStore = new TestInMemorySignalProtocolStore();
      PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);

      // Simply remove the pre-key information from a valid bundle
      bobPreKey =
          new PreKeyBundle(
              bobPreKey.getRegistrationId(),
              1,
              -1,
              null,
              bobPreKey.getSignedPreKeyId(),
              bobPreKey.getSignedPreKey(),
              bobPreKey.getSignedPreKeySignature(),
              bobPreKey.getIdentityKey(),
              bobPreKey.getKyberPreKeyId(),
              bobPreKey.getKyberPreKey(),
              bobPreKey.getKyberPreKeySignature());

      aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);

      assertTrue(aliceStore.containsSession(BOB_ADDRESS));
      assertTrue(aliceStore.loadSession(BOB_ADDRESS).getSessionVersion() == expectedVersion);

      String originalMessage = "Good, fast, cheap: pick two";
      SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
      CiphertextMessage outgoingMessage = aliceSessionCipher.encrypt(originalMessage.getBytes());

      assertTrue(outgoingMessage.getType() == CiphertextMessage.PREKEY_TYPE);

      PreKeySignalMessage incomingMessage = new PreKeySignalMessage(outgoingMessage.serialize());
      assertTrue(!incomingMessage.getPreKeyId().isPresent());

      SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);
      bobSessionCipher.decrypt(incomingMessage, UsePqRatchet.YES);

      assertTrue(bobStore.containsSession(ALICE_ADDRESS));
      assertEquals(bobStore.loadSession(ALICE_ADDRESS).getSessionVersion(), expectedVersion);

      SessionCipher bobSessionCipherForMallory = new SessionCipher(bobStore, MALLORY_ADDRESS);
      assertThrows(
          ReusedBaseKeyException.class,
          () -> bobSessionCipherForMallory.decrypt(incomingMessage, UsePqRatchet.YES));
    }
  }

  public static class VersionAgnostic {

    @Test
    public void testBadSignedPreKeySignature()
        throws InvalidKeyException, UntrustedIdentityException {
      SignalProtocolStore aliceStore = new TestInMemorySignalProtocolStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      IdentityKeyStore bobIdentityKeyStore = new TestInMemoryIdentityKeyStore();

      ECKeyPair bobPreKeyPair = ECKeyPair.generate();
      ECKeyPair bobSignedPreKeyPair = ECKeyPair.generate();
      byte[] bobSignedPreKeySignature =
          bobIdentityKeyStore
              .getIdentityKeyPair()
              .getPrivateKey()
              .calculateSignature(bobSignedPreKeyPair.getPublicKey().serialize());

      KEMKeyPair bobKyberPreKeyPair = KEMKeyPair.generate(KEMKeyType.KYBER_1024);
      byte[] bobKyberPreKeySignature =
          bobIdentityKeyStore
              .getIdentityKeyPair()
              .getPrivateKey()
              .calculateSignature(bobKyberPreKeyPair.getPublicKey().serialize());

      for (int i = 0; i < bobSignedPreKeySignature.length * 8; i++) {
        byte[] modifiedSignature = new byte[bobSignedPreKeySignature.length];
        System.arraycopy(
            bobSignedPreKeySignature, 0, modifiedSignature, 0, modifiedSignature.length);

        modifiedSignature[i / 8] ^= (0x01 << (i % 8));

        PreKeyBundle bobPreKey =
            new PreKeyBundle(
                bobIdentityKeyStore.getLocalRegistrationId(),
                1,
                31337,
                bobPreKeyPair.getPublicKey(),
                22,
                bobSignedPreKeyPair.getPublicKey(),
                modifiedSignature,
                bobIdentityKeyStore.getIdentityKeyPair().getPublicKey(),
                777,
                bobKyberPreKeyPair.getPublicKey(),
                bobKyberPreKeySignature);

        try {
          aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);
          fail("Accepted modified device key signature!");
        } catch (InvalidKeyException ike) {
          // good
        }
      }

      for (int i = 0; i < bobKyberPreKeySignature.length * 8; i++) {
        byte[] modifiedSignature = new byte[bobKyberPreKeySignature.length];
        System.arraycopy(
            bobKyberPreKeySignature, 0, modifiedSignature, 0, modifiedSignature.length);

        modifiedSignature[i / 8] ^= (0x01 << (i % 8));

        PreKeyBundle bobPreKey =
            new PreKeyBundle(
                bobIdentityKeyStore.getLocalRegistrationId(),
                1,
                31337,
                bobPreKeyPair.getPublicKey(),
                22,
                bobSignedPreKeyPair.getPublicKey(),
                bobSignedPreKeySignature,
                bobIdentityKeyStore.getIdentityKeyPair().getPublicKey(),
                777,
                bobKyberPreKeyPair.getPublicKey(),
                modifiedSignature);

        try {
          aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);
          fail("Accepted modified Kyber key signature!");
        } catch (InvalidKeyException ike) {
          // good
        }
      }

      PreKeyBundle bobPreKey =
          new PreKeyBundle(
              bobIdentityKeyStore.getLocalRegistrationId(),
              1,
              31337,
              bobPreKeyPair.getPublicKey(),
              22,
              bobSignedPreKeyPair.getPublicKey(),
              bobSignedPreKeySignature,
              bobIdentityKeyStore.getIdentityKeyPair().getPublicKey(),
              777,
              bobKyberPreKeyPair.getPublicKey(),
              bobKyberPreKeySignature);

      aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);
    }

    @Test
    public void testBadMessageBundle()
        throws InvalidKeyException,
            UntrustedIdentityException,
            InvalidVersionException,
            InvalidMessageException,
            DuplicateMessageException,
            LegacyMessageException,
            InvalidKeyIdException,
            NoSessionException {
      SignalProtocolStore aliceStore = new TestInMemorySignalProtocolStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      SignalProtocolStore bobStore = new TestInMemorySignalProtocolStore();
      BundleFactory bundleFactory = new PQXDHBundleFactory();
      PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);

      aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);

      String originalMessage = "Good, fast, cheap: pick two";
      SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
      CiphertextMessage outgoingMessageOne = aliceSessionCipher.encrypt(originalMessage.getBytes());

      assertTrue(outgoingMessageOne.getType() == CiphertextMessage.PREKEY_TYPE);

      byte[] goodMessage = outgoingMessageOne.serialize();
      byte[] badMessage = new byte[goodMessage.length];
      System.arraycopy(goodMessage, 0, badMessage, 0, badMessage.length);

      badMessage[badMessage.length - 10] ^= 0x01;

      PreKeySignalMessage incomingMessage = new PreKeySignalMessage(badMessage);
      SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);

      byte[] plaintext = new byte[0];

      try {
        plaintext = bobSessionCipher.decrypt(incomingMessage, UsePqRatchet.YES);
        fail("Decrypt should have failed!");
      } catch (InvalidMessageException e) {
        // good.
      }

      assertTrue(bobStore.containsPreKey(bobPreKey.getPreKeyId()));

      plaintext = bobSessionCipher.decrypt(new PreKeySignalMessage(goodMessage), UsePqRatchet.YES);

      assertTrue(originalMessage.equals(new String(plaintext)));
      assertFalse(bobStore.containsPreKey(bobPreKey.getPreKeyId()));
    }

    @Test
    public void testBadSignedPreKeyStore()
        throws InvalidKeyException,
            UntrustedIdentityException,
            InvalidVersionException,
            InvalidMessageException,
            DuplicateMessageException,
            LegacyMessageException,
            NoSessionException {
      SignalProtocolStore aliceStore = new TestNoSignedPreKeysStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      SignalProtocolStore bobStore = new TestNoSignedPreKeysStore();
      BundleFactory bundleFactory = new PQXDHBundleFactory();
      PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);

      aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);

      String originalMessage = "Good, fast, cheap: pick two";
      SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
      CiphertextMessage outgoingMessageOne = aliceSessionCipher.encrypt(originalMessage.getBytes());

      assertTrue(outgoingMessageOne.getType() == CiphertextMessage.PREKEY_TYPE);

      PreKeySignalMessage incomingMessage = new PreKeySignalMessage(outgoingMessageOne.serialize());
      SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);

      try {
        bobSessionCipher.decrypt(incomingMessage, UsePqRatchet.YES);
        fail("Decrypt should have failed!");
      } catch (InvalidKeyIdException e) {
        assertEquals(
            "TestNoSignedPreKeysStore rejected loading " + bobPreKey.getSignedPreKeyId(),
            e.getMessage());
      }
    }

    @Test
    public void testBadSignedPreKeyStoreError()
        throws InvalidKeyException,
            UntrustedIdentityException,
            InvalidVersionException,
            InvalidMessageException,
            DuplicateMessageException,
            LegacyMessageException,
            NoSessionException {
      SignalProtocolStore aliceStore = new TestBadSignedPreKeysStore();
      SessionBuilder aliceSessionBuilder = new SessionBuilder(aliceStore, BOB_ADDRESS);

      SignalProtocolStore bobStore = new TestBadSignedPreKeysStore();
      BundleFactory bundleFactory = new PQXDHBundleFactory();
      PreKeyBundle bobPreKey = bundleFactory.createBundle(bobStore);

      aliceSessionBuilder.process(bobPreKey, UsePqRatchet.YES);

      String originalMessage = "Good, fast, cheap: pick two";
      SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
      CiphertextMessage outgoingMessageOne = aliceSessionCipher.encrypt(originalMessage.getBytes());

      assertTrue(outgoingMessageOne.getType() == CiphertextMessage.PREKEY_TYPE);

      PreKeySignalMessage incomingMessage = new PreKeySignalMessage(outgoingMessageOne.serialize());
      SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);

      try {
        bobSessionCipher.decrypt(incomingMessage, UsePqRatchet.YES);
        fail("Decrypt should have failed!");
      } catch (InvalidKeyIdException e) {
        fail("libsignal swallowed the exception");
      } catch (TestBadSignedPreKeysStore.CustomException e) {
        // success!
      }
    }
  }

  private static void runInteraction(SignalProtocolStore aliceStore, SignalProtocolStore bobStore)
      throws DuplicateMessageException,
          LegacyMessageException,
          InvalidMessageException,
          InvalidVersionException,
          InvalidKeyException,
          NoSessionException,
          UntrustedIdentityException {
    SessionCipher aliceSessionCipher = new SessionCipher(aliceStore, BOB_ADDRESS);
    SessionCipher bobSessionCipher = new SessionCipher(bobStore, ALICE_ADDRESS);

    String originalMessage = "smert ze smert";
    CiphertextMessage aliceMessage = aliceSessionCipher.encrypt(originalMessage.getBytes());

    assertEquals(aliceMessage.getType(), CiphertextMessage.WHISPER_TYPE);

    byte[] plaintext = bobSessionCipher.decrypt(new SignalMessage(aliceMessage.serialize()));
    assertTrue(new String(plaintext).equals(originalMessage));

    CiphertextMessage bobMessage = bobSessionCipher.encrypt(originalMessage.getBytes());

    assertEquals(bobMessage.getType(), CiphertextMessage.WHISPER_TYPE);

    plaintext = aliceSessionCipher.decrypt(new SignalMessage(bobMessage.serialize()));
    assertTrue(new String(plaintext).equals(originalMessage));

    for (int i = 0; i < 10; i++) {
      String loopingMessage =
          ("What do we mean by saying that existence precedes essence? "
              + "We mean that man first of all exists, encounters himself, "
              + "surges up in the world--and defines himself aftward. "
              + i);
      CiphertextMessage aliceLoopingMessage = aliceSessionCipher.encrypt(loopingMessage.getBytes());

      byte[] loopingPlaintext =
          bobSessionCipher.decrypt(new SignalMessage(aliceLoopingMessage.serialize()));
      assertTrue(new String(loopingPlaintext).equals(loopingMessage));
    }

    for (int i = 0; i < 10; i++) {
      String loopingMessage =
          ("What do we mean by saying that existence precedes essence? "
              + "We mean that man first of all exists, encounters himself, "
              + "surges up in the world--and defines himself aftward. "
              + i);
      CiphertextMessage bobLoopingMessage = bobSessionCipher.encrypt(loopingMessage.getBytes());

      byte[] loopingPlaintext =
          aliceSessionCipher.decrypt(new SignalMessage(bobLoopingMessage.serialize()));
      assertTrue(new String(loopingPlaintext).equals(loopingMessage));
    }

    Set<Pair<String, CiphertextMessage>> aliceOutOfOrderMessages = new HashSet<>();

    for (int i = 0; i < 10; i++) {
      String loopingMessage =
          ("What do we mean by saying that existence precedes essence? "
              + "We mean that man first of all exists, encounters himself, "
              + "surges up in the world--and defines himself aftward. "
              + i);
      CiphertextMessage aliceLoopingMessage = aliceSessionCipher.encrypt(loopingMessage.getBytes());

      aliceOutOfOrderMessages.add(new Pair<>(loopingMessage, aliceLoopingMessage));
    }

    for (int i = 0; i < 10; i++) {
      String loopingMessage =
          ("What do we mean by saying that existence precedes essence? "
              + "We mean that man first of all exists, encounters himself, "
              + "surges up in the world--and defines himself aftward. "
              + i);
      CiphertextMessage aliceLoopingMessage = aliceSessionCipher.encrypt(loopingMessage.getBytes());

      byte[] loopingPlaintext =
          bobSessionCipher.decrypt(new SignalMessage(aliceLoopingMessage.serialize()));
      assertTrue(new String(loopingPlaintext).equals(loopingMessage));
    }

    for (int i = 0; i < 10; i++) {
      String loopingMessage = ("You can only desire based on what you know: " + i);
      CiphertextMessage bobLoopingMessage = bobSessionCipher.encrypt(loopingMessage.getBytes());

      byte[] loopingPlaintext =
          aliceSessionCipher.decrypt(new SignalMessage(bobLoopingMessage.serialize()));
      assertTrue(new String(loopingPlaintext).equals(loopingMessage));
    }

    for (Pair<String, CiphertextMessage> aliceOutOfOrderMessage : aliceOutOfOrderMessages) {
      byte[] outOfOrderPlaintext =
          bobSessionCipher.decrypt(new SignalMessage(aliceOutOfOrderMessage.second().serialize()));
      assertTrue(new String(outOfOrderPlaintext).equals(aliceOutOfOrderMessage.first()));
    }
  }
}
