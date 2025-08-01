//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

import Foundation
import SignalFfi

///
/// SgxClient provides bindings to interact with a Signal SGX service
///
/// Interaction with the service is done over a websocket, which is handled by the client.  Once the websocket
/// has been initiated, the client establishes a connection in the following manner:
///
/// <ul>
///     <li>connect to the service websocket, read service attestation message</li>
///     <li>instantiate SgxClient with the attestation message</li>
///     <li>send SgxClient.initialRequest()</li>
///     <li>receive a response and pass to SgxClient.completeHandshake()</li>
/// </ul>
///
/// After a connection has been established, a client may send or receive messages.  To send a message, they
/// formulate the plaintext, then pass it to SgxClient.establishedSend() to get the ciphertext message
/// to pass along.  When a message is received (as ciphertext), it is passed to Cds2Client.establishedRecv(),
/// which decrypts and verifies it, passing the plaintext back to the client for processing.
///
public class SgxClient: NativeHandleOwner<SignalMutPointerSgxClientState> {
    override internal class func destroyNativeHandle(
        _ handle: NonNull<SignalMutPointerSgxClientState>
    ) -> SignalFfiErrorRef? {
        return signal_sgx_client_state_destroy(handle.pointer)
    }

    /// Initial request to send to an SGX service, which begins post-attestation handshake.
    public func initialRequest() -> Data {
        return withNativeHandle { nativeHandle in
            failOnError {
                try invokeFnReturningData {
                    signal_sgx_client_state_initial_request($0, nativeHandle.const())
                }
            }
        }
    }

    /// Called by client upon receipt of first non-attestation message from service, to complete handshake.
    public func completeHandshake<Bytes: ContiguousBytes>(_ handshakeResponse: Bytes) throws {
        try withNativeHandle { nativeHandle in
            try handshakeResponse.withUnsafeBorrowedBuffer { buffer in
                try checkError(signal_sgx_client_state_complete_handshake(nativeHandle, buffer))
            }
        }
    }

    /// Called by client after completeHandshake has succeeded, to encrypt a message to send.
    public func establishedSend<Bytes: ContiguousBytes>(_ plaintextToSend: Bytes) throws -> Data {
        return try withNativeHandle { nativeHandle in
            try plaintextToSend.withUnsafeBorrowedBuffer { buffer in
                try invokeFnReturningData {
                    signal_sgx_client_state_established_send($0, nativeHandle, buffer)
                }
            }
        }
    }

    /// Called by client after completeHandshake has succeeded, to decrypt a received message.
    public func establishedRecv<Bytes: ContiguousBytes>(_ receivedCiphertext: Bytes) throws -> Data {
        return try withNativeHandle { nativeHandle in
            try receivedCiphertext.withUnsafeBorrowedBuffer { buffer in
                try invokeFnReturningData {
                    signal_sgx_client_state_established_recv($0, nativeHandle, buffer)
                }
            }
        }
    }
}

extension SignalMutPointerSgxClientState: SignalMutPointer {
    public typealias ConstPointer = SignalConstPointerSgxClientState

    public init(untyped: OpaquePointer?) {
        self.init(raw: untyped)
    }

    public func toOpaque() -> OpaquePointer? {
        self.raw
    }

    public func const() -> Self.ConstPointer {
        Self.ConstPointer(raw: self.raw)
    }
}

extension SignalConstPointerSgxClientState: SignalConstPointer {
    public func toOpaque() -> OpaquePointer? {
        self.raw
    }
}
