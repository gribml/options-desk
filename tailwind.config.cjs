module.exports = {
  content: ['./index.html', './src/**/*.rs'],
  theme: {
    extend: {
      colors: {
        surface: '#0f1117',
        panel: '#1a1d27',
        border: '#2a2d3a',
      },
      fontFamily: { mono: ["'JetBrains Mono'", 'monospace'] },
    },
  },
};
