# The oracle for rubystrftime: Ruby 2.6's Time#strftime on
# Time.at(sec, nsec, :nsec).utc. The format is hex of UTF-8, '-' when
# empty; the answer is hex of the text, or E:<class>:<hex message>.
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
  begin
    out << hx(Time.at(f[0].to_i, f[1].to_i, :nsec).utc.strftime(unhex(f[2])).b)
  rescue ArgumentError => e
    out << "E:#{e.class}:#{hx(e.message.b)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
