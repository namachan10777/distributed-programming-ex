project('intro', 'c',
  version : '0.1',
  default_options : ['warning_level=3'])

mercury = dependency('mercury')
margo = dependency('margo')

server = executable('server', 'server.c',
  install : true,
  dependencies: [mercury, margo])


client = executable('client', 'client.c',
  install: true,
  dependencies: [mercury, margo])