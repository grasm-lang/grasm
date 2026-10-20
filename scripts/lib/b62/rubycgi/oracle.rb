# The oracle for rubycgi: Ruby 2.6's CGI helpers (with the cgi/escape C
# extension), CGI.parse, CGI::Cookie and ERB::Util. Bytes are hex ('-'
# when empty); 'n' is nil.
require 'cgi'
require 'erb'

def unhex(h)
  (h == '-' ? '' : [h].pack('H*')).force_encoding('UTF-8')
end

def hx(s)
  s.empty? ? '-' : s.b.unpack1('H*')
end

def pairs(h)
  h.empty? ? '~' : h.map { |k, vs| hx(k) + '=' + vs.map { |v| hx(v) }.join('.') }.join(',')
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    out << case f[0]
           when 'es' then hx(CGI.escape(unhex(f[1])))
           when 'ue' then hx(ERB::Util.url_encode(unhex(f[1])))
           when 'un' then hx(CGI.unescape(unhex(f[1])))
           when 'eh' then hx(CGI.escapeHTML(unhex(f[1])))
           when 'hh' then hx(ERB::Util.html_escape(unhex(f[1])))
           when 'uh' then hx(CGI.unescapeHTML(unhex(f[1])))
           when 'ee' then hx(CGI.escapeElement(unhex(f[1]), *f[2..-1].map { |x| unhex(x) }))
           when 'ce' then hx(CGI.unescapeElement(unhex(f[1]), *f[2..-1].map { |x| unhex(x) }))
           when 'pr' then hx(CGI.pretty(unhex(f[1]), unhex(f[2])))
           when 'pa' then pairs(CGI.parse(unhex(f[1])))
           when 'cp' then pairs(CGI::Cookie.parse(unhex(f[1])).map { |k, c| [k, c.value] })
           when 'ct'
             opts = { 'name' => unhex(f[1]), 'value' => f[2] == '~' ? [] : f[2].split('.').map { |x| unhex(x) }, 'path' => unhex(f[4]) }
             opts['domain'] = unhex(f[3]) unless f[3] == 'n'
             opts['expires'] = Time.at(f[5].to_i) unless f[5] == 'n'
             opts['secure'] = true if f[6] == '1'
             opts['httponly'] = true if f[7] == '1'
             hx(CGI::Cookie.new(opts).to_s)
           else hx(CGI.rfc1123_date(Time.at(f[1].to_i)))
           end
  rescue ArgumentError => e
    out << "E:#{e.class}:#{hx(e.message)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
