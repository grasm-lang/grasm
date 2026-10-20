# The oracle for rubyabbrev: Ruby 2.6's Abbrev.abbrev, with no pattern or
# a String pattern (taken as a prefix). Words are hex of UTF-8, '-' when
# empty; the answer is the hash in insertion order, key=value pairs.
require 'abbrev'

def unhex(h)
  (h == '-' ? '' : [h].pack('H*')).force_encoding('UTF-8')
end

def hx(s)
  s.empty? ? '-' : s.unpack1('H*')
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  pattern = f[0] == '~' ? nil : unhex(f[0])
  words = f[1..-1].map { |x| unhex(x) }
  t = Abbrev.abbrev(words, pattern)
  out << (t.empty? ? '~' : t.map { |k, v| hx(k) + '=' + hx(v) }.join(','))
end
STDOUT.write(out.map { |o| o + "\n" }.join)
