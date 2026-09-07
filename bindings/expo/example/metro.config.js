const path = require('node:path');
const { getDefaultConfig } = require('expo/metro-config');
const config = getDefaultConfig(__dirname);
config.watchFolders = [path.resolve(__dirname, '..')];
config.resolver.assetExts.push('wasm');
// One React/Expo runtime across the local SDK link, without disabling nested
// dependency lookup (Expo legitimately installs some dependencies below itself).
config.resolver.resolveRequest = (context, name, platform) => {
  if (/^(react|react-native|expo|expo-modules-core)(\/|$)/.test(name)) {
    context = { ...context, originModulePath: path.join(__dirname, 'index.js') };
  }
  return context.resolveRequest(context, name, platform);
};
module.exports = config;
