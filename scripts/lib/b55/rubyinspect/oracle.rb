# The oracle for packages/rubyinspect: Ruby 2.6's String#inspect, #dump
# and #undump, run with UTF-8 as the default external encoding (-E). A
# line is "i HEX B", "d HEX B" (B=1 for a binary string) or "u HEX"; the
# answer is the result's bytes in hex ("-" when empty), for undump with
# "|" and the encoding's name, or E: and the error's message.
def unhex(h)
  h == "-" ? "".b : [h].pack("H*")
end

def hex(s)
  h = s.unpack("H*")[0]
  h.empty? ? "-" : h
end

out = []
STDIN.each_line do |l|
  f = l.split
  next if f.empty?
  s = unhex(f[1])
  r =
    case f[0]
    when "i"
      hex((f[2] == "1" ? s : s.force_encoding("UTF-8")).inspect)
    when "d"
      hex((f[2] == "1" ? s : s.force_encoding("UTF-8")).dump)
    else
      begin
        u = s.force_encoding("UTF-8").undump
        hex(u) + "|" + u.encoding.name
      rescue RuntimeError => e
        "E:" + e.message
      end
    end
  out << r
end
STDOUT.write(out.map { |x| x + "\n" }.join)
