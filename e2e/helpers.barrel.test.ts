// e2e/helpers.barrel.test.ts
//
// Build-time coverage of the helpers.ts barrel. Asserts every name imported
// by any un-migrated spec is still re-exported. If you remove a name from
// helpers/{peers,ui,touch}.ts, tsc fails here with TS2305 before any
// Playwright test runs.
//
// This is NOT a Playwright spec (filename uses .test.ts so Playwright's
// default `testMatch: '*.spec.ts'` skips it). It executes only as part of
// `npx tsc --noEmit` / `npx eslint`.

import {
  // peers
  freshStart,
  createServer,
  getPeerId,
  generateInvite,
  joinViaInvite,
  setupTwoPeers,
  waitForApp,
  openServerSettings,
  // ui
  sendMessage,
  waitForMessage,
  switchChannel,
  createChannel,
  openSidebar,
  closeSidebar,
  openMemberList,
  closeMemberList,
  visibleShell,
  isMobile,
  messageAction,
  editMessage,
  deleteMessage,
  reactToMessage,
  trustPeer,
  untrustPeer,
  kickPeer,
  openCompareFingerprints,
  markFingerprintsMatch,
  markFingerprintsMismatch,
  getMessages,
  switchTab,
  // touch
  longPress,
  longPressAvatar,
  swipeLeft,
  swipeRight,
} from './helpers';

// One reference per name so TS can't tree-shake the imports away. The
// `void` operator silences `@typescript-eslint/no-unused-expressions`
// without needing an eslint-disable comment.
void freshStart;
void createServer;
void getPeerId;
void generateInvite;
void joinViaInvite;
void setupTwoPeers;
void waitForApp;
void openServerSettings;
void sendMessage;
void waitForMessage;
void switchChannel;
void createChannel;
void openSidebar;
void closeSidebar;
void openMemberList;
void closeMemberList;
void visibleShell;
void isMobile;
void messageAction;
void editMessage;
void deleteMessage;
void reactToMessage;
void trustPeer;
void untrustPeer;
void kickPeer;
void openCompareFingerprints;
void markFingerprintsMatch;
void markFingerprintsMismatch;
void getMessages;
void switchTab;
void longPress;
void longPressAvatar;
void swipeLeft;
void swipeRight;
