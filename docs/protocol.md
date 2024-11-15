# Beelay - A new sync protocol for Automerge

This document describes a new sync protocol - tentatively named "Beelay" - designed for efficiently synchronizing collections of Automerge documents. 

## Introduction and motivation

An Automerge document can be thought of as like git for JSON. Each document is a data structure which can be materialized as a JSON object, but modifications to the document are stored within the document as a commit DAG rather like Git. This means that Automerge documents can be edited concurrently and merged together automatically later, providing a substrate for collaboration without central peers.

One common requirement for collaborative software is real-time communication between concurrently editing peers. For this purpose Automerge includes a sync protocol which enables peers to send deltas rather than sending the entire document on every change - for things like per-keystroke editing this is essential. This sync protocol works but production use has exposed a few limitations: firstly, that running sync servers is expensive and compromises security and secondly, that applications frequently want to synchronize many documents.

### Sync Servers

Due to the structure of networks on the internet, and due to requirements for data availability, it is often necessary to have a central server which acts as a relay and store-and-forward peer for sync messages. Unlike peers running on behalf of a single user these peers must synchronize many documents at once. The existing sync protocol requires that the entire document be in memory in order to run the sync protocol which limits the scale these servers can affordably reach.

In addition, one of the appealing things about Automerge is that you don't need to trust a central server. Introducing a sync server somewhat compromises this feature - you must trust the server operator not to look at your data and to keep it secure.

### Collections of Documents

An Automerge document is a "unit of sharing". You can't share subcomponents of a document with other people - or view the history of just one part of the document. This is a useful property because it makes the behavior of a document predictable in the face of concurrent changes but it is also a limitation because many useful applications involve sharing subsets of a users data with different people.

To work around this limitation applications often create many Automerge documents and link them together via a location independent url. This introduces challenges for the sync protocol. We have users who have thousands of documents - synchronizing just the small number of documents which have changed in one session is very inefficient with the current scheme - which must load every document into memory and create a separate sync session for each one.

### Requirements

These two sets of problems are technically unrelated, what brings them together for our purposes is that they can both be solved by a new sync protocol. Specifically what we require is a sync protocol which

* Does not impose O(n) memory requirements on sync servers where n is the number of documents being synchronized    
* Allows for sync servers to operate over encrypted data to reduce the trust users have to place in sync servers
* Provides a mechanism for efficiently determining what documents in a collection of documents have changed

## Design

Beelay is an RPC based protocol which operates between two peers. We make no assumptions about the ordering of messages on the channel connecting these two peers. In typical interactions one peer will be a "sync server" and one will be a user agent within some application, although the protocol doesn't require this topology.

To begin synchronizing a document with a remote peer the local Beelay peer first requests that the remote peer create a "snapshot" representing the current state of the given document and every document transitively reachable (via links) from it. The remote peer creates this snapshot and returns an identifier the local peer can use to perform [RIBLT-sync] with the snapshot. Once this sync is complete the local peer knows which documents in the collection are out of sync.

At this point the local peer runs [ sedimentree sync ]( ./sedimentree.md ) for each out of sync document. Once this is complete the local peer knows that it is at least up to date with the state of the collection at the time of the snapshot. Finally the local peer can [listen] to any changes to documents in the snapshot which have occurred since the snapshot was created. Thus the local peer can stay in sync with live updates.

It must be possible to run this synchronization protocol over encrypted data which means that we cannot directly examine the contents of the documents being synchronzed in order to extract the links between them. Instead, peers which have the clear text of the documents synchronize the links between documents to a "reachability index" on the server. This index is a very simple CRDT which is also synchronized via [sedimentree sync].

### RIBLT Sync

RIBLT sync refers to "Rateless Invertible Bloom Lookup Tables" as presented in [Practical Rateless Set Reconciliation](https://arxiv.org/abs/2402.02668). This scheme allows for a set of items to be reconciled between two peers with a bandwidth overhead which is proportional to the size of the set difference and with a very low number of round trips.

### Sedimentree Sync

Sedimentree sync is a scheme we have designed for synchronizing commit DAGs such as those which make up an Automerge document. The important features of sedimentree sync are that it allows for compressing runs of operations in the commit DAG _and ommitting their change hashes_ from the compressed runs. This is crucial as it allows us to keep a very granular commit history without using enormous amounts of storage and/or bandwidth. See [sedimentree.md] for more details. 

## Messages

Beelay messages are encoded in a binary format which we describe here. With the following additional notation:

```
byte = %x00-FF
        ; any octet
bytes = 1*byte
        ; any sequence of octets
uleb128 = 1*8( %x00-7F ) / ( %x80-FF bytes )
        ; unsigned LEB128 encoding
leb128 = 1*8( %x00-7F ) / ( %x80-FF bytes )
        ; signed LEB128 encoding
```    

With the exception of notifications sent in response to listen requests, every message is either a request or a response. 

```
message = message_type ((request_id (request / response)) /  notification)
message_type = %d00 ; request
    / %d01 ; response
    / %d02 ; notification
request_id = 16(byte)

request = create_snapshot_request
    / snapshot_symbols_request
    / fetch_sedimentree_request
    / fetch_blob_part_request
    / upload_commits_request
    / upload_blob_request
    / listen
response = create_snapshot
    / snapshot_symbols_response
    / fetch_sedimentree_response
    / fetch_blob_part_response
    / upload_commits_response
    / upload_blob_response
    / listen
```

### Create Snapshot

The create snapshot request contains just the document ID of the root document to create a snapshot from.

```
create_snapshot_request = %d04 ; request type
    uleb128 ; length of root document ID
    bytes ; root document ID
```

The response to a create snapshot request contains the snapshot ID and the first "coded symbols" from the RIBLT sync for the snapshot in question. The receiver should attempt to peel these symbols and if they find they still need more, then use the `snapshot_symbols` request to request more symbols.

```
create_snapshot_response = %d04 ; response type
    16(byte) ; snapshot ID
    uleb128 ; number of coded symbols
    *coded_symbol

coded_symbol = 16(byte) ; first part of symbol which is a document ID when peeled
    32(byte) ; second part of symbol, which is the hash of the heads of the 
             ; document when peeled
```


### Snapshot Symbols

Used to request additional RIBLT coded symbols for an existing snapshot.

```
snapshot_symbols_request = %d05 ; request type
    16(byte) ; snapshot ID

snapshot_symbols_response = %d05 ; response type
    uleb128 ; number of coded symbols
    *coded_symbol
```

### Fetch Sedimentree

Request to fetch the Sedimentree (both content and reachability index) for a specific document.

```
fetch_sedimentree_request = %d01 ; request type
    uleb128 ; length of document ID
    bytes ; document ID

fetch_sedimentree_response = %d02 ; response type
    sedimentree_data

sedimentree_data = %d00 ; not found
    / (%d01 ; found
       content_summary
       index_summary)

content_summary = uleb128 ; number of bundles
    *sedimentree_bundle

index_summary = uleb128 ; number of bundles  
    *sedimentree_bundle

sedimentree_bundle = bytes ; bundle data
```

### Fetch Blob Part

Request to fetch a portion of a blob by its hash.

```
fetch_blob_part_request = %d02 ; request type
    32(byte) ; blob hash
    uleb128 ; offset
    uleb128 ; length

fetch_blob_part_response = %d03 ; response type
    uleb128 ; length of blob part
    bytes ; blob part data
```

### Upload Commits

Request to upload commits for a document.

```
upload_commits_request = %d00 ; request type
    uleb128 ; length of document ID
    bytes ; document ID
    commit_category ; single byte indicating commit category
    uleb128 ; number of upload items
    *upload_item

upload_commits_response = %d01 ; response type
    ; empty response indicating success

upload_item = blob_ref tree_part

blob_ref = %d00 32(byte) ; blob hash
    / (%d01 ; inline blob
       uleb128 ; blob length
       bytes) ; blob data

tree_part = %d00 ; stratum
    (%d00 / (%d01 32(byte))) ; optional start commit hash
    32(byte) ; end commit hash
    uleb128 ; number of checkpoints
    *32(byte) ; checkpoint hashes
    / (%d01 ; commit
       32(byte) ; commit hash
       uleb128 ; number of parents
       *32(byte)) ; parent hashes
```

### Upload Blob

Request to upload a complete blob.

```
upload_blob_request = %d03 ; request type
    uleb128 ; blob length
    bytes ; blob data

upload_blob_response = %d01 ; response type
    ; empty response indicating success
```

### Listen

Request to listen for updates to a snapshot. Any changes to the documents reachable from the snapshot which have been discovered by the server since the snapshot was created will be delivered as "notification" messages, which are the only kind of message sent without an explicit request.

```
listen_request = %d06 ; request type
    16(byte) ; snapshot ID

listen_response = %d06 ; response type
    ; empty response indicating successful subscription

notification = peer_id
    doc_id
    upload_item

peer_id = uleb128 ; length of peer ID
    bytes ; peer ID bytes

doc_id = uleb128 ; length of document ID
    bytes ; document ID bytes
```

### Notifications

Notifications are sent to a peer when a change is discovered to a document which is transitively reachable from a snapshot the peer issued a `listen` request to.


```
notification = from_peer
    document_id
    upload_item

from_peer = uleb128 ; length of peer ID bytes
    bytes ; peer ID

document_id = uleb128 ; length of document ID bytes
    bytes ; document ID

upload_item = blob_ref tree_part

blob_ref = %d00 32(byte) ; blob hash reference
    / (%d01 ; inline blob
       uleb128 ; blob length
       bytes) ; blob data

tree_part = %d00 ; stratum
    (%d00 / (%d01 32(byte))) ; optional start commit hash
    32(byte) ; end commit hash
    uleb128 ; number of checkpoints
    *32(byte) ; checkpoint hashes
    / (%d01 ; commit
       32(byte) ; commit hash
       uleb128 ; number of parents
       *32(byte)) ; parent hashes
```


### Error Response

Any request can result in an error response:

```
error_response = %d00 ; error response type
    uleb128 ; length of error message
    bytes ; UTF-8 encoded error message
```

