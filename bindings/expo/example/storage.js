// Synthetic fixture credentials only. Native proof uses the real simulator
// Keychain; Web proof awaits a whole IndexedDB transaction, not just a request.
import { Platform } from 'react-native';
import * as SecureStore from 'expo-secure-store';

const key = 'auki.zitadel.todo.z09';
async function webTransaction(mode, operation) {
  const db = await new Promise((resolve, reject) => {
    const opening = indexedDB.open('auki-zitadel-todo-z09', 1);
    opening.onupgradeneeded = () => opening.result.createObjectStore('credentials');
    opening.onsuccess = () => resolve(opening.result);
    opening.onerror = () => reject(new Error('fixture store unavailable'));
  });
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction('credentials', mode, mode === 'readwrite' ? { durability: 'strict' } : undefined);
      const request = operation(tx.objectStore('credentials'));
      tx.oncomplete = () => resolve(request.result ?? null);
      tx.onabort = tx.onerror = () => reject(new Error('fixture store transaction failed'));
    });
  } finally { db.close(); }
}
export const durable = {
  save: value => Platform.OS === 'web'
    ? webTransaction('readwrite', store => store.put(JSON.stringify(value), key))
    : SecureStore.setItemAsync(key, JSON.stringify(value), { keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY }),
  load: async () => {
    const json = Platform.OS === 'web'
      ? await webTransaction('readonly', store => store.get(key))
      : await SecureStore.getItemAsync(key);
    return json ? JSON.parse(json) : null;
  },
  clear: () => Platform.OS === 'web'
    ? webTransaction('readwrite', store => store.delete(key))
    : SecureStore.deleteItemAsync(key),
};
